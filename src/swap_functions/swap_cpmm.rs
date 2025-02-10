use crate::swap_functions::new_signed_and_send::*;
use crate::swap_functions::utils::{deserialize_anchor_account, get_transfer_inverse_fee};
use anchor_client::{Client, Cluster};
use anyhow::{anyhow, Result, Context};
use arrayref::array_ref;
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::program_pack::Pack;
use solana_sdk::{
    pubkey::Pubkey,
    signer::{keypair::Keypair, Signer},
    system_instruction
};
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account;
use spl_token_2022::{
    state::{Account, Mint},
    extension::StateWithExtensionsMut,
};
use std::{env, str::FromStr, sync::Arc};
use tokio::task;
use raydium_cp_swap::{AUTH_SEED, accounts as raydium_cp_accounts, instruction as raydium_cp_instructions};
use cpswap_cli::swap_calculate;

pub async fn swap_cpmm(
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
    
    // Build instructions vector
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let url = Cluster::Custom(rpc_url, ws_url.to_string());
    let keypair_arc = Arc::new(keypair);
    let anchor_client = Client::new(url.clone(), keypair_arc.clone());
    let program_id_str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
    let program_id = Pubkey::from_str(program_id_str)?;
    let program = anchor_client.program(program_id)?;
    let pool_state_result = task::spawn_blocking(move || {
        program.account(pool_pubkey) // Blocking call
    }).await;
    match pool_state_result {
        Ok(Ok(pool_state)) => {
            let pool_state: raydium_cp_swap::states::PoolState = pool_state;
            // ... process pool_state ...
            let load_pubkeys = vec![
                pool_state.amm_config,
                pool_state.token_0_vault,
                pool_state.token_1_vault,
                pool_state.token_0_mint,
                pool_state.token_1_mint,
                token_ata.clone(),   
            ];

            let sol_balance = blocking_client
            .get_token_account_balance(&pool_state.token_1_vault)?;


            let token_balance = blocking_client
                .get_token_account_balance(&pool_state.token_0_vault)?;

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

            let token_balance = blocking_client
                .get_token_account_balance(&token_ata)?
                .amount
                .parse::<u64>()?;
            if token_balance == 0 {
                return Err(anyhow!("No tokens available to swap"));
            }
            // Use the entire token balance
            let amount_raw = token_balance;

            // Calculate minimum amount out (you may want to adjust this based on your requirements)
            let _amount_out = amount_raw;

            let rsps = blocking_client.get_multiple_accounts(&load_pubkeys)?;
            let epoch = blocking_client.get_epoch_info().unwrap().epoch;
            let [amm_config_account, token_0_vault_account, token_1_vault_account, token_0_mint_account, token_1_mint_account, user_input_token_account] =
                array_ref![rsps, 0, 6];

            // docode account
            let mut token_0_vault_data = token_0_vault_account.clone().unwrap().data;
            let mut token_1_vault_data = token_1_vault_account.clone().unwrap().data;
            let mut token_0_mint_data = token_0_mint_account.clone().unwrap().data;
            let mut token_1_mint_data = token_1_mint_account.clone().unwrap().data;
            let mut user_input_token_data = user_input_token_account.clone().unwrap().data;
            let amm_config_state = deserialize_anchor_account::<raydium_cp_swap::states::AmmConfig>(
                amm_config_account.as_ref().unwrap(),
            )?;

            let token_0_vault_info = StateWithExtensionsMut::<Account>::unpack(&mut token_0_vault_data)?;
            let token_1_vault_info = StateWithExtensionsMut::<Account>::unpack(&mut token_1_vault_data)?;
            let token_0_mint_info = StateWithExtensionsMut::<Mint>::unpack(&mut token_0_mint_data)?;
            let token_1_mint_info = StateWithExtensionsMut::<Mint>::unpack(&mut token_1_mint_data)?;
            let user_input_token_info_result = StateWithExtensionsMut::<Account>::unpack(&mut user_input_token_data);
            match user_input_token_info_result {
                Ok(user_input_token_info) => {
                    let (total_token_0_amount, total_token_1_amount) = pool_state.vault_amount_without_fee(
                        token_0_vault_info.base.amount,
                        token_1_vault_info.base.amount,
                    );
                    let amount_out_less_fee = swap_calculate(&blocking_client, pool_pubkey, token_ata.clone(), amount_raw, slippage, true)?.other_amount_threshold;
                    let (
                        trade_direction,
                        total_input_token_amount,
                        total_output_token_amount,
                        user_input_token,
                        _user_output_token,
                        input_vault,
                        output_vault,
                        input_token_mint,
                        output_token_mint,
                        input_token_program,
                        output_token_program,
                        out_transfer_fee,
                    ) = if user_input_token_info.base.mint == token_0_vault_info.base.mint {
                        (
                            raydium_cp_swap::curve::TradeDirection::ZeroForOne,
                            total_token_0_amount,
                            total_token_1_amount,
                            token_ata.clone(),
                            spl_associated_token_account::get_associated_token_address_with_program_id(
                                &keypair_arc.pubkey(),
                                &pool_state.token_1_mint,
                                &spl_token_2022::id(),
                            ),
                            pool_state.token_0_vault,
                            pool_state.token_1_vault,
                            pool_state.token_0_mint,
                            pool_state.token_1_mint,
                            pool_state.token_0_program,
                            pool_state.token_1_program,
                            get_transfer_inverse_fee(&token_1_mint_info, epoch, amount_out_less_fee),
                        )
                    } else {
                        (
                            raydium_cp_swap::curve::TradeDirection::OneForZero,
                            total_token_1_amount,
                            total_token_0_amount,
                            token_ata.clone(),
                            spl_associated_token_account::get_associated_token_address_with_program_id(
                                &keypair_arc.pubkey(),
                                &pool_state.token_0_mint,
                                &spl_token::id(),
                            ),
                            pool_state.token_1_vault,
                            pool_state.token_0_vault,
                            pool_state.token_1_mint,
                            pool_state.token_0_mint,
                            pool_state.token_1_program,
                            pool_state.token_0_program,
                            get_transfer_inverse_fee(&token_0_mint_info, epoch, amount_out_less_fee),
                        )
                    };

                    let actual_amount_out = amount_out_less_fee.checked_add(out_transfer_fee).unwrap();
        
                    let result = raydium_cp_swap::curve::CurveCalculator::swap_base_output(
                        u128::from(actual_amount_out),
                        u128::from(total_input_token_amount),
                        u128::from(total_output_token_amount),
                        amm_config_state.trade_fee_rate,
                        amm_config_state.protocol_fee_rate,
                        amm_config_state.fund_fee_rate,
                    )
                    .ok_or(raydium_cp_swap::error::ErrorCode::ZeroTradingTokens)
                    .unwrap();
        
                    let source_amount_swapped = u64::try_from(result.source_amount_swapped).unwrap();
                    let _amount_in_transfer_fee = match trade_direction {
                        raydium_cp_swap::curve::TradeDirection::ZeroForOne => {
                            get_transfer_inverse_fee(&token_0_mint_info, epoch, source_amount_swapped)
                        }
                        raydium_cp_swap::curve::TradeDirection::OneForZero => {
                            get_transfer_inverse_fee(&token_1_mint_info, epoch, source_amount_swapped)
                        }
                    };

                    let mut instructions = Vec::new();

                    let create_instr = Some(create_associated_token_account(
                        &keypair_arc.clone().pubkey(),
                        &keypair_arc.clone().pubkey(),
                        &pool_state.token_0_mint,
                        &spl_token::ID
                    ));
                    if let Some(create_instr) = create_instr {
                        instructions.push(create_instr);
                    }
                    let seed = &format!("{}", Keypair::new().pubkey())[..32];
                    let wsol_pubkey = Pubkey::create_with_seed(&owner, seed, &spl_token::id())?;
                    let rent = nonblocking_client
                        .get_minimum_balance_for_rent_exemption(Account::LEN)
            .           await?;
                    instructions.push(system_instruction::create_account_with_seed(
                        &keypair_arc.clone().pubkey(),
                        &wsol_pubkey,
                        &keypair_arc.clone().pubkey(),
                        seed,
                        rent,
                        Account::LEN as u64,
                        &spl_token::id(),
                    ));
                    let native_mint = spl_token::native_mint::ID;
                    instructions.push(spl_token::instruction::initialize_account(
                        &spl_token::id(),
                        &wsol_pubkey,
                        &native_mint,
                        &keypair_arc.clone().pubkey(),
                    )?);

                    let program_id_str_instr = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
                    let program_id_instr = Pubkey::from_str(&program_id_str_instr)?;
                    let program_instr = anchor_client.program(program_id_instr)?;
                    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program_instr.id());
                    let swap_base_in_instr = program_instr
                        .request()
                        .accounts(raydium_cp_accounts::Swap {
                            payer: program_instr.payer(),
                            authority,
                            amm_config: pool_state.amm_config,
                            pool_state: pool_pubkey,
                            input_token_account: user_input_token,
                            output_token_account: wsol_pubkey,
                            input_vault,
                            output_vault,
                            input_token_program,
                            output_token_program,
                            input_token_mint,
                            output_token_mint,
                            observation_state: pool_state.observation_key,
                        })
                        .args(raydium_cp_instructions::SwapBaseOutput {
                            max_amount_in: amount_raw,
                            amount_out: amount_out_less_fee,
                        })
                        .instructions()?;
                    instructions.extend(swap_base_in_instr);
                    let in_ata = get_associated_token_address(&keypair_arc.clone().pubkey(), &input_token_mint);
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
                    let close_instruction = Some(spl_token::instruction::close_account(
                        &spl_token::ID,
                        &in_ata,
                        &keypair_arc.clone().pubkey(),
                        &keypair_arc.clone().pubkey(),
                        &vec![&keypair_arc.clone().pubkey()],
                    )?);
                    if let Some(close_instruction) = close_instruction {
                        instructions.push(close_instruction);
                    }
                    new_signed_and_send(&blocking_client, &keypair_arc.clone(), instructions, use_jito).await
                },
                Err(e) => {
                    println!("CPMM Swap Program Error");
                    return Err(anyhow!("Failed to unpack user input token account: {}", e));
                }
            }
            
        }
        Ok(Err(_err)) => {
            // Handle the error from Program::account
            let instructions = Vec::new();
            println!("CPMM Swap Client Error");
            new_signed_and_send(&blocking_client, &keypair_arc, instructions, use_jito).await
        }
        Err(_err) => {
            // Handle the error from spawn_blocking
            let instructions = Vec::new();
            println!("CPMM Swap Join Error");
            new_signed_and_send(&blocking_client, &keypair_arc, instructions, use_jito).await
        }
    }
}

pub async fn get_pool_price_cpmm(  
    pool_id: Option<&str>,  
    keypair: Keypair,  
    blocking_client: Arc<RpcClient>,  
) -> Result<f64> {      
    // Assuming similar logic to fetch pool state here  
    let pool_pubkey = Pubkey::from_str(pool_id.ok_or_else(|| anyhow!("Pool ID is required"))?)?;  
        
    // Similar logic to fetch pool state (could also call swap_cpmm again if only pool_price is needed)  
    let program_id_str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";  
    let program_id = Pubkey::from_str(program_id_str)?;  
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");  
    let ws_url = "wss://api.mainnet-beta.solana.com/";  
    let url = Cluster::Custom(rpc_url, ws_url.to_string());  
    let keypair_arc = Arc::new(keypair);  
    let anchor_client = Client::new(url.clone(), keypair_arc.clone());  
    let program = anchor_client.program(program_id)?;  

    let pool_state_result = task::spawn_blocking(move || {  
        program.account(pool_pubkey) // Blocking call  
    }).await;  

    match pool_state_result {  
        Ok(Ok(pool_state)) => {  
            let pool_state: raydium_cp_swap::states::PoolState = pool_state;  

            let sol_balance = blocking_client  
                .get_token_account_balance(&pool_state.token_1_vault)?;  
                
            let token_balance = blocking_client  
                .get_token_account_balance(&pool_state.token_0_vault)?;  

            // Convert amounts  
            let sol_amount = sol_balance  
                .amount  
                .parse::<f64>()?;  

            let token_amount = token_balance  
                .amount  
                .parse::<f64>()?;  
                
            let pool_price = token_amount / sol_amount; // Calculate pool price  

            Ok(pool_price) // Return the calculated pool price  
        },  
        Ok(Err(err)) => {  
            Err(anyhow!("Failed to fetch pool state: {}", err))  
        },  
        Err(_) => {  
            Err(anyhow!("Task failed while fetching pool state."))  
        },  
    }  
} 
