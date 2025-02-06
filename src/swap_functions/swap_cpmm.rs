use crate::new_signed_and_send;
use crate::swap_functions::utils::{deserialize_anchor_account, get_transfer_inverse_fee, amount_with_slippage, calculate_amount_out_less_fee, get_fee_rates};
use crate::swap_functions::instructions::{create_ata_token_account_instr, swap_base_output_instr};
use anchor_client::{Client, Cluster};
use anyhow::{anyhow, Result};
use arrayref::array_ref;
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signer::{keypair::Keypair, Signer},
};
use spl_associated_token_account::get_associated_token_address;
use spl_token_2022::{
    extension::StateWithExtensionsMut,
    state::{Account, Mint},
};
use std::env;
use std::str::FromStr;
use std::sync::Arc;

pub async fn swap_cpmm(
    pool_id: Option<&str>,
    keypair: Keypair,
    mint_str: &str,
    slippage: u64,
    use_jito: bool,
    blocking_client: Arc<RpcClient>,
    _nonblocking_client: Arc<NonblockingRpcClient>,
) -> Result<Vec<String>> {
    let owner = keypair.pubkey();
    let mint = Pubkey::from_str(mint_str)?;
    let pool_pubkey = Pubkey::from_str(pool_id.ok_or_else(|| anyhow!("Pool ID is required"))?)?;

    // Get token account balance
    let token_ata = get_associated_token_address(&owner, &mint);
    let token_balance = blocking_client
        .get_token_account_balance(&token_ata)?
        .amount
        .parse::<u64>()?;

    if token_balance == 0 {
        return Err(anyhow!("No tokens available to swap"));
    }
    // Use the entire token balance
    let amount_raw = token_balance;
    let max_amount_in = amount_raw + (amount_raw * slippage as u64) / 100;

    // Calculate minimum amount out (you may want to adjust this based on your requirements)
    let _amount_out = amount_raw; // This should ideally be calculated based on pool state and price impact

    // Build instructions vector
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let url = Cluster::Custom(rpc_url, ws_url.to_string());
    let keypair_arc = Arc::new(keypair);
    let anchor_client = Client::new(url, keypair_arc.clone());
    let program_id_str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
    let program_id = Pubkey::from_str(program_id_str)?;
    let program = anchor_client.program(program_id)?;
    let pool_state: raydium_cp_swap::states::PoolState = program.account(pool_pubkey)?;

    let load_pubkeys = vec![
        pool_state.amm_config,
        pool_state.token_0_vault,
        pool_state.token_1_vault,
        pool_state.token_0_mint,
        pool_state.token_1_mint,
        mint,
    ];

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
    let user_input_token_info =
        StateWithExtensionsMut::<Account>::unpack(&mut user_input_token_data)?;

    let (total_token_0_amount, total_token_1_amount) = pool_state.vault_amount_without_fee(
        token_0_vault_info.base.amount,
        token_1_vault_info.base.amount,
    );

    let (trade_fee_rate, protocol_fee_rate, fund_fee_rate) = get_fee_rates(&blocking_client, &pool_pubkey)?;

    // Calculate the amount out less fee
    let amount_out_less_fee = calculate_amount_out_less_fee(
        max_amount_in,
        trade_fee_rate,
        protocol_fee_rate,
        fund_fee_rate,
    );

    let (
        trade_direction,
        total_input_token_amount,
        total_output_token_amount,
        user_input_token,
        user_output_token,
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
            mint,
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
            mint,
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &keypair_arc.pubkey(),
                &pool_state.token_0_mint,
                &spl_token_2022::id(),
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
    let amount_in_transfer_fee = match trade_direction {
        raydium_cp_swap::curve::TradeDirection::ZeroForOne => {
            get_transfer_inverse_fee(&token_0_mint_info, epoch, source_amount_swapped)
        }
        raydium_cp_swap::curve::TradeDirection::OneForZero => {
            get_transfer_inverse_fee(&token_1_mint_info, epoch, source_amount_swapped)
        }
    };

    let input_transfer_amount = source_amount_swapped
        .checked_add(amount_in_transfer_fee)
        .unwrap();
    // calc max in with slippage
    let max_amount_in = amount_with_slippage(input_transfer_amount, slippage as f64, true);
    let mut instructions = Vec::new();
    let create_user_output_token_instr = create_ata_token_account_instr(
        (*keypair_arc).insecure_clone(),
        spl_token::id(),
        &output_token_mint,
        &owner,
    )?;
    instructions.extend(create_user_output_token_instr);
    let swap_base_in_instr = swap_base_output_instr(
        (*keypair_arc).insecure_clone(),
        pool_pubkey,
        pool_state.amm_config,
        pool_state.observation_key,
        user_input_token,
        user_output_token,
        input_vault,
        output_vault,
        input_token_mint,
        output_token_mint,
        input_token_program,
        output_token_program,
        max_amount_in,
        amount_out_less_fee,
    )?;
    instructions.extend(swap_base_in_instr);

    // Add the swap instruction
    println!("Swapping {} tokens (raw amount)", amount_raw);

    // Add instruction to close the token account after swap
    let close_instruction = spl_token::instruction::close_account(
        &spl_token::ID,
        &token_ata,
        &owner,
        &owner,
        &[&owner],
    )?;
    instructions.push(close_instruction);

    new_signed_and_send(&blocking_client, &keypair_arc, instructions, use_jito).await
}
