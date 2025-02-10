use crate::swap_functions::new_signed_and_send::*;
use anchor_client::{Client, Cluster};
use anchor_lang::prelude::*;
use anyhow::{anyhow, Result, Context};
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    account::Account, program_pack::Pack, pubkey::Pubkey, signer::{keypair::Keypair, Signer}, system_instruction,
};
use spl_associated_token_account::get_associated_token_address;
use spl_token_2022::{
    extension::StateWithExtensions,
    state::Account as Account2022,
};
use std::{collections::VecDeque, env, rc::Rc, str::FromStr, sync::Arc};
use anchor_lang::AccountDeserialize;
use arrayref::array_ref;
use std::ops::{DerefMut, Neg, Mul};

use raydium_amm_v3::{
    libraries::*,
    states::{PoolState, TickArrayBitmapExtension, TickArrayState, AmmConfig, TickState},
};
use raydium_amm_v3::accounts as raydium_accounts;
use raydium_amm_v3::instruction as raydium_instruction;

pub async fn swap_clmm(
    pool_id: Option<&str>,
    keypair: Keypair,
    mint_str: &str,
    slippage: u64,
    use_jito: bool,
    blocking_client: Arc<RpcClient>,
    nonblocking_client: Arc<NonblockingRpcClient>,
) -> Result<Vec<String>> {
    let owner = keypair.pubkey();
    let mint = Pubkey::from_str(mint_str)?;
    let pool_pubkey = Pubkey::from_str(pool_id.ok_or_else(|| anyhow!("Pool ID is required"))?)?;

    // Get token account balance
    let token_ata = get_associated_token_address(&owner, &mint);
    let w_ata = get_associated_token_address(&spl_token::ID, &spl_token::native_mint::ID);

    // Build instructions vector
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let keypair_byte = keypair.to_bytes();
    let keypair_clone = Keypair::from_bytes(&keypair_byte).expect("Failed to clone keypair");
    let keypair_arc = Arc::new(&keypair_clone);
    let amm_config_index = 2 as u16;
    let (amm_config_key, __bump) = Pubkey::find_program_address(
        &[
            raydium_amm_v3::states::AMM_CONFIG_SEED.as_bytes(),
            &amm_config_index.to_be_bytes(),
        ],
        &raydium_amm_v3::ID,
    );
    let input_token = token_ata;
    let output_token = w_ata;
    let limit_price = None;
    let base_in = true;
    let mint0 = Some(mint);
    let mint1 = Some(spl_token::native_mint::ID);
    let payer_byte = keypair.to_bytes();
    let payer = Keypair::from_bytes(&payer_byte).expect("Failed to clone keypair");
    let pool_id_account = Some(pool_pubkey.clone());
    let tickarray_bitmap_extension = Some(Pubkey::from_str("9z9VTNvaTpJuwjn4LSnjHwZgUR9iGuy59BwXTNbxRF6s")?);
    let pool_config = ClientConfig {
        http_url: rpc_url.clone(),
        ws_url: ws_url.to_string(),
        raydium_v3_program: raydium_amm_v3::ID,
        slippage: slippage as f64,
        amm_config_key: amm_config_key,
        mint0: mint0,
        mint1: mint1,
        pool_id_account: pool_id_account,
        tickarray_bitmap_extension: tickarray_bitmap_extension
    };
    let load_accounts = vec![
        input_token,    
        output_token,
        pool_config.amm_config_key,
        pool_config.pool_id_account.unwrap(),
        pool_config.tickarray_bitmap_extension.unwrap(),
    ];
    let rsps = blocking_client.get_multiple_accounts(&load_accounts)?;
    let [user_input_account, user_output_account, amm_config_account, pool_account, tickarray_bitmap_extension_account] =
        array_ref![rsps, 0, 5];
    let pool_state = deserialize_anchor_account::<raydium_amm_v3::states::PoolState>(
        pool_account.as_ref().unwrap(),
    )?;
    let sol_balance = blocking_client
        .get_token_account_balance(&pool_state.token_vault_1)?;


    let token_balance = blocking_client
        .get_token_account_balance(&pool_state.token_vault_0)?;

    // Convert amounts
    let sol_amount = sol_balance
        .amount
        .parse::<f64>()?;

    let token_amount = token_balance
        .amount
        .parse::<f64>()?;
    let pool_price = token_amount / sol_amount;
    let target_price_str =
        env::var("TARGET_PRICE").context("TARGET_ADDRESS environment variable not set")?;
    let target_price: f64 = target_price_str  
        .parse::<f64>()  
        .context("Failed to parse TARGET_PRICE as f64")?; 
    if pool_price < target_price {
        println!("Current price is lower than target price");
        return Err(anyhow!("Current price is lower than target price"));
    }
    let user_input_state =
        StateWithExtensions::<Account2022>::unpack(&user_input_account.as_ref().unwrap().data)
            .unwrap();   
    let user_output_state =
        StateWithExtensions::<Account2022>::unpack(&user_output_account.as_ref().unwrap().data)
            .unwrap();
    let amm_config_state = deserialize_anchor_account::<raydium_amm_v3::states::AmmConfig>(
        amm_config_account.as_ref().unwrap(),
    )?;
    let token_balance_wallet = blocking_client
        .get_token_account_balance(&token_ata)?
        .amount
        .parse::<u64>()?;
    if token_balance_wallet == 0 {
        return Err(anyhow!("No tokens available to swap"));
    }
    // Use the entire token balance
    let amount_raw = token_balance_wallet;

    let amount = amount_raw as u64;

    let tickarray_bitmap_extension =
        deserialize_anchor_account::<raydium_amm_v3::states::TickArrayBitmapExtension>(
            tickarray_bitmap_extension_account.as_ref().unwrap(),
        )?;
    let zero_for_one = user_input_state.base.mint == pool_state.token_mint_0
        && user_output_state.base.mint == pool_state.token_mint_1;
    println!("Token Mint {:?} : {:?} : {:?} : {:?}", user_input_state.base.mint, user_output_state.base.mint, pool_state.token_mint_0, pool_state.token_mint_1);
    // load tick_arrays
    let mut tick_arrays = load_cur_and_next_five_tick_array(
        &blocking_client,
        &pool_config,
        &pool_state,
        &tickarray_bitmap_extension,
        zero_for_one,
    );

    let mut sqrt_price_limit_x64 = None;
    if limit_price.is_some() {
        let sqrt_price_x64 = price_to_sqrt_price_x64(
            limit_price.unwrap(),
            pool_state.mint_decimals_0,
            pool_state.mint_decimals_1,
        );
        sqrt_price_limit_x64 = Some(sqrt_price_x64);
    }
    let (mut other_amount_threshold, mut tick_array_indexs) =
        get_out_put_amount_and_remaining_accounts(
            amount,
            sqrt_price_limit_x64,
            zero_for_one,
            base_in,
            &amm_config_state,
            &pool_state,
            &tickarray_bitmap_extension,
            &mut tick_arrays,
        )
        .unwrap();
    if base_in {
        // min out
        other_amount_threshold =
            amount_with_slippage(other_amount_threshold, pool_config.slippage, false);
    } else {
        // max in
        other_amount_threshold =
            amount_with_slippage(other_amount_threshold, pool_config.slippage, true);
    }

    let current_or_next_tick_array_key = Pubkey::find_program_address(
        &[
            raydium_amm_v3::states::TICK_ARRAY_SEED.as_bytes(),
            pool_config.pool_id_account.unwrap().to_bytes().as_ref(),
            &tick_array_indexs.pop_front().unwrap().to_be_bytes(),
        ],
        &pool_config.raydium_v3_program,
    )
    .0;
    let mut remaining_accounts = Vec::new();
    let mut accounts = tick_array_indexs
        .into_iter()
        .map(|index| {
            AccountMeta::new(
                Pubkey::find_program_address(
                    &[
                        raydium_amm_v3::states::TICK_ARRAY_SEED.as_bytes(),
                        pool_config.pool_id_account.unwrap().to_bytes().as_ref(),
                        &index.to_be_bytes(),
                    ],
                    &pool_config.raydium_v3_program,
                )
                .0,
                false,
            )
        })
        .collect();
    remaining_accounts.append(&mut accounts);
    let mut instructions = Vec::new();

    let url = Cluster::Custom(pool_config.http_url.clone(), pool_config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer));
    let program = client.program(pool_config.raydium_v3_program)?;

    let seed = &format!("{}", Keypair::new().pubkey())[..32];
    let wsol_pubkey = Pubkey::create_with_seed(&owner, seed, &spl_token::id())?;
    let rent = nonblocking_client
        .get_minimum_balance_for_rent_exemption(Account2022::LEN)
.           await?;
    let swap_instr = program
        .request()
        .accounts(raydium_accounts::SwapSingle {
            payer: program.payer(),
            amm_config: pool_state.amm_config,
            pool_state: pool_config.pool_id_account.unwrap(),
            input_token_account: input_token,
            output_token_account: wsol_pubkey,
            input_vault: if zero_for_one {
                pool_state.token_vault_0
            } else {
                pool_state.token_vault_1
            },
            output_vault: if zero_for_one {
                pool_state.token_vault_1
            } else {
                pool_state.token_vault_0
            },
            tick_array: current_or_next_tick_array_key,
            observation_state: pool_state.observation_key,
            token_program: spl_token::id(),
        })
        .accounts(remaining_accounts)
        .args(raydium_instruction::Swap {
            amount,
            other_amount_threshold,
            sqrt_price_limit_x64: sqrt_price_limit_x64.unwrap_or(0u128),
            is_base_input: base_in,
        })
        .instructions()?;
    instructions.push(system_instruction::create_account_with_seed(
        &keypair_arc.clone().pubkey(),
        &wsol_pubkey,
        &keypair_arc.clone().pubkey(),
        seed,
        rent,
        Account2022::LEN as u64, // 165, // Token account size
        &spl_token::id(),
    ));
    let native_mint = spl_token::native_mint::ID;
    instructions.push(spl_token::instruction::initialize_account(
        &spl_token::id(),
        &wsol_pubkey,
        &native_mint,
        &keypair_arc.clone().pubkey(),
    )?);

    instructions.extend(swap_instr);
    let _in_ata = get_associated_token_address(&keypair_arc.clone().pubkey(), &mint0.unwrap());
    let close_wsol_account_instruction = Some(spl_token::instruction::close_account(
        &spl_token::ID,
        &wsol_pubkey,
        &keypair_arc.clone().pubkey(),
        &keypair_arc.clone().pubkey(),
        &vec![&keypair_arc.clone().pubkey()],
    )?);
    if let Some(close_wsol_account_instruction) = close_wsol_account_instruction {
        instructions.push(close_wsol_account_instruction);
    }
    println!("FInal");
    new_signed_and_send(&blocking_client, &keypair_clone, instructions, use_jito).await
}


#[derive(Debug, PartialEq)]
pub struct ClientConfig {
    http_url: String,
    ws_url: String,
    raydium_v3_program: Pubkey,
    slippage: f64,
    amm_config_key: Pubkey,
    mint0: Option<Pubkey>,
    mint1: Option<Pubkey>,
    pool_id_account: Option<Pubkey>,
    tickarray_bitmap_extension: Option<Pubkey>,
}

#[derive(Debug)]
pub struct SwapState {
    // the amount remaining to be swapped in/out of the input/output asset
    pub amount_specified_remaining: u64,
    // the amount already swapped out/in of the output/input asset
    pub amount_calculated: u64,
    // current sqrt(price)
    pub sqrt_price_x64: u128,
    // the tick associated with the current price
    pub tick: i32,
    // the current liquidity in range
    pub liquidity: u128,
}

#[derive(Default)]
struct StepComputations {
    // the price at the beginning of the step
    sqrt_price_start_x64: u128,
    // the next tick to swap to from the current tick in the swap direction
    tick_next: i32,
    // whether tick_next is initialized or not
    initialized: bool,
    // sqrt(price) for the next tick (1/0)
    sqrt_price_next_x64: u128,
    // how much is being swapped in in this step
    amount_in: u64,
    // how much is being swapped out
    amount_out: u64,
    // how much fee is being paid in
    fee_amount: u64,
}

fn deserialize_anchor_account<T: AccountDeserialize>(account: &Account) -> Result<T> {
    let mut data: &[u8] = &account.data;
    T::try_deserialize(&mut data).map_err(Into::into)
}

fn load_cur_and_next_five_tick_array(
    rpc_client: &RpcClient,
    pool_config: &ClientConfig,
    pool_state: &PoolState,
    tickarray_bitmap_extension: &TickArrayBitmapExtension,
    zero_for_one: bool,
) -> VecDeque<TickArrayState> {
    let (_, mut current_vaild_tick_array_start_index) = pool_state
        .get_first_initialized_tick_array(&Some(*tickarray_bitmap_extension), zero_for_one)
        .unwrap();
    let mut tick_array_keys = Vec::new();
    tick_array_keys.push(
        Pubkey::find_program_address(
            &[
                raydium_amm_v3::states::TICK_ARRAY_SEED.as_bytes(),
                pool_config.pool_id_account.unwrap().to_bytes().as_ref(),
                &current_vaild_tick_array_start_index.to_be_bytes(),
            ],
            &pool_config.raydium_v3_program,
        )
        .0,
    );
    let mut max_array_size = 5;
    while max_array_size != 0 {
        let next_tick_array_index = pool_state
            .next_initialized_tick_array_start_index(
                &Some(*tickarray_bitmap_extension),
                current_vaild_tick_array_start_index,
                zero_for_one,
            )
            .unwrap();
        if next_tick_array_index.is_none() {
            break;
        }
        current_vaild_tick_array_start_index = next_tick_array_index.unwrap();
        tick_array_keys.push(
            Pubkey::find_program_address(
                &[
                    raydium_amm_v3::states::TICK_ARRAY_SEED.as_bytes(),
                    pool_config.pool_id_account.unwrap().to_bytes().as_ref(),
                    &current_vaild_tick_array_start_index.to_be_bytes(),
                ],
                &pool_config.raydium_v3_program,
            )
            .0,
        );
        max_array_size -= 1;
    }
    let tick_array_rsps = rpc_client.get_multiple_accounts(&tick_array_keys).unwrap();
    let mut tick_arrays = VecDeque::new();
    for tick_array in tick_array_rsps {
        let tick_array_state =
            deserialize_anchor_account::<raydium_amm_v3::states::TickArrayState>(
                &tick_array.unwrap(),
            )
            .unwrap();
        tick_arrays.push_back(tick_array_state);
    }
    tick_arrays
}

pub fn price_to_sqrt_price_x64(price: f64, decimals_0: u8, decimals_1: u8) -> u128 {
    let price_with_decimals = price * multipler(decimals_1) / multipler(decimals_0);
    price_to_x64(price_with_decimals.sqrt())
}

pub fn multipler(decimals: u8) -> f64 {
    (10_i32).checked_pow(decimals.try_into().unwrap()).unwrap() as f64
}

pub fn price_to_x64(price: f64) -> u128 {
    (price * fixed_point_64::Q64 as f64) as u128
}

pub fn get_out_put_amount_and_remaining_accounts(
    input_amount: u64,
    sqrt_price_limit_x64: Option<u128>,
    zero_for_one: bool,
    is_base_input: bool,
    pool_config: &AmmConfig,
    pool_state: &PoolState,
    tickarray_bitmap_extension: &TickArrayBitmapExtension,
    tick_arrays: &mut VecDeque<TickArrayState>,
) -> Result<(u64, VecDeque<i32>), &'static str> {
    let (is_pool_current_tick_array, current_vaild_tick_array_start_index) = pool_state
        .get_first_initialized_tick_array(&Some(*tickarray_bitmap_extension), zero_for_one)
        .unwrap();

    let (amount_calculated, tick_array_start_index_vec) = swap_compute(
        zero_for_one,
        is_base_input,
        is_pool_current_tick_array,
        pool_config.trade_fee_rate,
        input_amount,
        current_vaild_tick_array_start_index,
        sqrt_price_limit_x64.unwrap_or(0),
        pool_state,
        tickarray_bitmap_extension,
        tick_arrays,
    )?;

    Ok((amount_calculated, tick_array_start_index_vec))
}

fn swap_compute(
    zero_for_one: bool,
    is_base_input: bool,
    is_pool_current_tick_array: bool,
    fee: u32,
    amount_specified: u64,
    current_vaild_tick_array_start_index: i32,
    sqrt_price_limit_x64: u128,
    pool_state: &PoolState,
    tickarray_bitmap_extension: &TickArrayBitmapExtension,
    tick_arrays: &mut VecDeque<TickArrayState>,
) -> Result<(u64, VecDeque<i32>), &'static str> {
    if amount_specified == 0 {
        return Result::Err("amountSpecified must not be 0");
    }
    let sqrt_price_limit_x64 = if sqrt_price_limit_x64 == 0 {
        if zero_for_one {
            tick_math::MIN_SQRT_PRICE_X64 + 1
        } else {
            tick_math::MAX_SQRT_PRICE_X64 - 1
        }
    } else {
        sqrt_price_limit_x64
    };
    if zero_for_one {
        if sqrt_price_limit_x64 < tick_math::MIN_SQRT_PRICE_X64 {
            return Result::Err("sqrt_price_limit_x64 must greater than MIN_SQRT_PRICE_X64");
        }
        if sqrt_price_limit_x64 >= pool_state.sqrt_price_x64 {
            return Result::Err("sqrt_price_limit_x64 must smaller than current");
        }
    } else {
        if sqrt_price_limit_x64 > tick_math::MAX_SQRT_PRICE_X64 {
            return Result::Err("sqrt_price_limit_x64 must smaller than MAX_SQRT_PRICE_X64");
        }
        if sqrt_price_limit_x64 <= pool_state.sqrt_price_x64 {
            return Result::Err("sqrt_price_limit_x64 must greater than current");
        }
    }
    let mut tick_match_current_tick_array = is_pool_current_tick_array;

    let mut state = SwapState {
        amount_specified_remaining: amount_specified,
        amount_calculated: 0,
        sqrt_price_x64: pool_state.sqrt_price_x64,
        tick: pool_state.tick_current,
        liquidity: pool_state.liquidity,
    };

    let mut tick_array_current = tick_arrays.pop_front().unwrap();
    if tick_array_current.start_tick_index != current_vaild_tick_array_start_index {
        return Result::Err("tick array start tick index does not match");
    }
    let mut tick_array_start_index_vec = VecDeque::new();
    tick_array_start_index_vec.push_back(tick_array_current.start_tick_index);
    let mut loop_count = 0;
    // loop across ticks until input liquidity is consumed, or the limit price is reached
    while state.amount_specified_remaining != 0
        && state.sqrt_price_x64 != sqrt_price_limit_x64
        && state.tick < tick_math::MAX_TICK
        && state.tick > tick_math::MIN_TICK
    {
        if loop_count > 10 {
            return Result::Err("loop_count limit");
        }
        let mut step = StepComputations::default();
        step.sqrt_price_start_x64 = state.sqrt_price_x64;
        // save the bitmap, and the tick account if it is initialized
        let mut next_initialized_tick = if let Some(tick_state) = tick_array_current
            .next_initialized_tick(state.tick, pool_state.tick_spacing, zero_for_one)
            .unwrap()
        {
            Box::new(*tick_state)
        } else {
            if !tick_match_current_tick_array {
                tick_match_current_tick_array = true;
                Box::new(
                    *tick_array_current
                        .first_initialized_tick(zero_for_one)
                        .unwrap(),
                )
            } else {
                Box::new(TickState::default())
            }
        };
        if !next_initialized_tick.is_initialized() {
            let current_vaild_tick_array_start_index = pool_state
                .next_initialized_tick_array_start_index(
                    &Some(*tickarray_bitmap_extension),
                    current_vaild_tick_array_start_index,
                    zero_for_one,
                )
                .unwrap();
            tick_array_current = tick_arrays.pop_front().unwrap();
            if current_vaild_tick_array_start_index.is_none() {
                return Result::Err("tick array start tick index out of range limit");
            }
            if tick_array_current.start_tick_index != current_vaild_tick_array_start_index.unwrap()
            {
                return Result::Err("tick array start tick index does not match");
            }
            tick_array_start_index_vec.push_back(tick_array_current.start_tick_index);
            let mut first_initialized_tick = tick_array_current
                .first_initialized_tick(zero_for_one)
                .unwrap();

            next_initialized_tick = Box::new(*first_initialized_tick.deref_mut());
        }
        step.tick_next = next_initialized_tick.tick;
        step.initialized = next_initialized_tick.is_initialized();
        if step.tick_next < MIN_TICK {
            step.tick_next = MIN_TICK;
        } else if step.tick_next > MAX_TICK {
            step.tick_next = MAX_TICK;
        }

        step.sqrt_price_next_x64 = tick_math::get_sqrt_price_at_tick(step.tick_next).unwrap();

        let target_price = if (zero_for_one && step.sqrt_price_next_x64 < sqrt_price_limit_x64)
            || (!zero_for_one && step.sqrt_price_next_x64 > sqrt_price_limit_x64)
        {
            sqrt_price_limit_x64
        } else {
            step.sqrt_price_next_x64
        };
        let swap_step = swap_math::compute_swap_step(
            state.sqrt_price_x64,
            target_price,
            state.liquidity,
            state.amount_specified_remaining,
            fee,
            is_base_input,
            zero_for_one,
            1,
        )
        .unwrap();
        state.sqrt_price_x64 = swap_step.sqrt_price_next_x64;
        step.amount_in = swap_step.amount_in;
        step.amount_out = swap_step.amount_out;
        step.fee_amount = swap_step.fee_amount;

        if is_base_input {
            state.amount_specified_remaining = state
                .amount_specified_remaining
                .checked_sub(step.amount_in + step.fee_amount)
                .unwrap();
            state.amount_calculated = state
                .amount_calculated
                .checked_add(step.amount_out)
                .unwrap();
        } else {
            state.amount_specified_remaining = state
                .amount_specified_remaining
                .checked_sub(step.amount_out)
                .unwrap();
            state.amount_calculated = state
                .amount_calculated
                .checked_add(step.amount_in + step.fee_amount)
                .unwrap();
        }

        if state.sqrt_price_x64 == step.sqrt_price_next_x64 {
            // if the tick is initialized, run the tick transition
            if step.initialized {
                let mut liquidity_net = next_initialized_tick.liquidity_net;
                if zero_for_one {
                    liquidity_net = liquidity_net.neg();
                }
                state.liquidity =
                    liquidity_math::add_delta(state.liquidity, liquidity_net).unwrap();
            }

            state.tick = if zero_for_one {
                step.tick_next - 1
            } else {
                step.tick_next
            };
        } else if state.sqrt_price_x64 != step.sqrt_price_start_x64 {
            // recompute unless we're on a lower tick boundary (i.e. already transitioned ticks), and haven't moved
            state.tick = tick_math::get_tick_at_sqrt_price(state.sqrt_price_x64).unwrap();
        }
        loop_count += 1;
    }

    Ok((state.amount_calculated, tick_array_start_index_vec))
}

pub fn amount_with_slippage(amount: u64, slippage: f64, round_up: bool) -> u64 {
    if round_up {
        (amount as f64).mul(1_f64 + slippage).ceil() as u64
    } else {
        (amount as f64).mul(1_f64 - slippage).floor() as u64
    }
}