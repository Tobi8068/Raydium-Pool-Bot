use crate::get_account_info;
use crate::swap_functions::new_signed_and_send::*;
use crate::swap_functions::utils::{deserialize_anchor_account, get_transfer_inverse_fee, calculate_amount_out_less_fee};
use anchor_client::{Client, Cluster};
use anyhow::{anyhow, Result};
use arrayref::array_ref;
use common::token;
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::program_pack::Pack;
use solana_sdk::{
    pubkey::Pubkey,
    signer::{keypair::Keypair, Signer},
    transaction::Transaction,
    system_instruction::create_account,
    sysvar::{rent::Rent}
};
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account;
use spl_token::{
    state::Mint as SPL_MINT,
    instruction::initialize_account,
};
use spl_token_2022::{
    state::{Account, Mint},
    extension::StateWithExtensionsMut,
};
use std::{env, rc::Rc};
use std::str::FromStr;
use std::sync::Arc;
use tokio::task;
use core::mem::size_of;
use raydium_cp_swap::AUTH_SEED;
use raydium_cp_swap::accounts as raydium_cp_accounts;
use raydium_cp_swap::instruction as raydium_cp_instructions;
use cpswap_cli::swap_calculate;


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
    println!("TokenAccount=>>>>>>>>>>>>> {:?}", &token_ata);
    let token_balance = blocking_client
        .get_token_account_balance(&token_ata)?
        .amount
        .parse::<u64>()?;

    if token_balance == 0 {
        return Err(anyhow!("No tokens available to swap"));
    }
    // Use the entire token balance
    let amount_raw = token_balance;
    let max_amount_in = amount_raw - (amount_raw * slippage as u64) / 100;
    println!("{} {} {}", r"----".repeat(5), max_amount_in, amount_raw);

    // Calculate minimum amount out (you may want to adjust this based on your requirements)
    let _amount_out = amount_raw; // This should ideally be calculated based on pool state and price impact

    // Build instructions vector
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let url = Cluster::Custom(rpc_url, ws_url.to_string());
    let keypair_arc = Arc::new(keypair);
    // let anchor_client = Client::new(url.clone(), keypair_arc.clone());
    let anchor_client = Client::new(url.clone(), keypair_arc.clone());
    let program_id_str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
    let program_id = Pubkey::from_str(program_id_str)?;
    let program = anchor_client.program(program_id)?;
    let pool_state_result = task::spawn_blocking(move || {
        program.account(pool_pubkey) // Blocking call
    }).await;
    // let pool_state: raydium_cp_swap::states::PoolState = program.account(pool_pubkey)?;
    match pool_state_result {
        Ok(Ok(pool_state)) => {
            println!("Get Pool State ...");
            let pool_state: raydium_cp_swap::states::PoolState = pool_state;
            println!("Debugging step0 ... ");
            // ... process pool_state ...
            let load_pubkeys = vec![
                pool_state.amm_config,
                pool_state.token_0_vault,
                pool_state.token_1_vault,
                pool_state.token_0_mint,
                pool_state.token_1_mint,
                token_ata.clone(),   
            ];

            println!("MintKey:  {:?}\n", load_pubkeys);

            let rsps = blocking_client.get_multiple_accounts(&load_pubkeys)?;
            let epoch = blocking_client.get_epoch_info().unwrap().epoch;
            let [amm_config_account, token_0_vault_account, token_1_vault_account, token_0_mint_account, token_1_mint_account, user_input_token_account] =
                array_ref![rsps, 0, 6];

            println!("User Input token: {:?} {:?}", user_input_token_account, token_0_vault_account);

            // docode account
            let mut token_0_vault_data = token_0_vault_account.clone().unwrap().data;
            let mut token_1_vault_data = token_1_vault_account.clone().unwrap().data;
            let mut token_0_mint_data = token_0_mint_account.clone().unwrap().data;
            let mut token_1_mint_data = token_1_mint_account.clone().unwrap().data;
            let mut user_input_token_data = user_input_token_account.clone().unwrap().data;
            let amm_config_state = deserialize_anchor_account::<raydium_cp_swap::states::AmmConfig>(
                amm_config_account.as_ref().unwrap(),
            )?;


            eprintln!("Token Data: >>>>>>>>>>>>>>, {:?}", &token_0_vault_account);
            
            println!("Debugging step1 ... ");
            
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
                    // // Calculate the amount out less fee
                    // let amount_out_less_fee = calculate_amount_out_less_fee(
                    //     max_amount_in,
                    //     amm_config_state.trade_fee_rate,
                    //     amm_config_state.protocol_fee_rate,
                    //     amm_config_state.fund_fee_rate,
                    // );
                    let amount_out_less_fee = swap_calculate(&blocking_client, pool_pubkey, token_ata.clone(), amount_raw, 30, true)?.other_amount_threshold;
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
                        println!(">>>>>>>>>>>>>>>>>>>>ZeroForOne>>>>>>>>>>>>>>>>>>>>>>");
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
                        println!(">>>>>>>>>>>>>>>>>>>>OneForZero>>>>>>>>>>>>>>>>>>>>>>");
                        let target_wata = Pubkey::from_str("EDENvG1tc9oeJyJP1tGGD3bL8fJ9CH3p6BpeNwnFu52E")?;

                        (
                            raydium_cp_swap::curve::TradeDirection::OneForZero,
                            total_token_1_amount,
                            total_token_0_amount,
                            token_ata.clone(),
                            Pubkey::from_str("EDENvG1tc9oeJyJP1tGGD3bL8fJ9CH3p6BpeNwnFu52E")?,
                            pool_state.token_1_vault,
                            pool_state.token_0_vault,
                            pool_state.token_1_mint,
                            pool_state.token_0_mint,
                            pool_state.token_1_program,
                            pool_state.token_0_program,
                            get_transfer_inverse_fee(&token_0_mint_info, epoch, amount_out_less_fee),
                        )
                    };

                    println!("Out token>>>>>>>>>>>>>>>{:?}", &user_output_token);
                    
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
        
                    println!("Debugging step3 ...");
        
                    // let input_transfer_amount = source_amount_swapped
                    //     .checked_add(amount_in_transfer_fee)
                    //     .unwrap();
                    // calc max in with slippage
                    
                    // let max_amount_in = amount_with_slippage(input_transfer_amount, slippage as f64, true);
                    let mut instructions = Vec::new();
                    // let create_user_output_token_instr = create_ata_token_account_instr(
                    //     &anchor_client,
                    //     spl_token::id(),
                    //     &output_token_mint,
                    //     &owner,
                    // ).await?;
                    

                    let program_token = anchor_client.program(spl_token::id())?;

                    // let create_instr = create_associated_token_account(
                    //     &keypair_arc.clone().pubkey(),
                    //     &user_output_token,
                    //     &pool_state.token_0_mint,
                    //     &program_token.id()
                    // );
                    // instructions.push(create_instr);
                    // let initialize_instr = initialize_account(
                    //     &spl_token::id(),
                    //     &user_output_token,
                    //     &pool_state.token_0_mint,
                    //     &keypair_arc.clone().pubkey()
                    // )?;
                    // instructions.push(initialize_instr);

                    // let create_user_output_token_instr = program_token
                    //     .request()
                    //     .instruction(
                    //         spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                    //                 &program_token.payer(),
                    //                 &owner,
                    //                 &output_token_mint,
                    //                 &spl_token::id()
                    //             )
                    //     )
                    //     .instructions()?;
            
                    println!("Debugging step4 ...");

                    // instructions.extend(create_user_output_token_instr);

                    let program_id_str_instr = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
                    let program_id_instr = Pubkey::from_str(&program_id_str_instr)?;
                    let program_instr = anchor_client.program(program_id_instr)?;
                    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program_instr.id());
                    // let swap_base_in_instr = swap_base_output_instr(
                    //     &anchor_client,
                    //     pool_pubkey,
                    //     pool_state.amm_config,
                    //     pool_state.observation_key,
                    //     user_input_token,
                    //     user_output_token,
                    //     input_vault,
                    //     output_vault,
                    //     input_token_mint,
                    //     output_token_mint,
                    //     input_token_program,
                    //     output_token_program,
                    //     max_amount_in,
                    //     amount_out_less_fee,
                    // )?;
                    
                    let swap_base_in_instr = program_instr
                        .request()
                        .accounts(raydium_cp_accounts::Swap {
                            payer: program_instr.payer(),
                            authority,
                            amm_config: pool_state.amm_config,
                            pool_state: pool_pubkey,
                            input_token_account: user_input_token,
                            output_token_account: user_output_token,
                            input_vault,
                            output_vault,
                            input_token_program,
                            output_token_program,
                            input_token_mint,
                            output_token_mint,
                            observation_state: pool_state.observation_key,
                        })
                        .args(raydium_cp_instructions::SwapBaseOutput {
                            amount_raw,
                            amount_out: amount_out_less_fee,
                        })
                        .instructions()?;
                    instructions.extend(swap_base_in_instr);
                    println!(">>>>>>>>>>>>>>OUTPUT & INPUT>>>>>>>>>>>>>>, {:?} {:?}", max_amount_in, amount_out_less_fee);
                    println!("Debugging step5...");
            
        
                    // Add the swap instruction
                    println!("Swapping {} tokens (raw amount)", amount_raw);
        
                    // Add instruction to close the token account after swap
                    // let close_instruction = spl_token::instruction::close_account(
                    //     &spl_token::ID,
                    //     &token_ata,
                    //     &owner,
                    //     &owner,
                    //     &[&owner],
                    // )?;
                    // instructions.push(close_instruction);
                    println!("Debugging step6...");

                    // let unit_limit = get_unit_limit();
                    // let unit_price = get_unit_price();
                    // let modify_compute_units =
                    //     solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(
                    //         unit_limit,
                    //     );
                    // let add_priority_fee =
                    //     solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(
                    //         unit_price,
                    //     );
                    // instructions.insert(0, modify_compute_units);
                    // instructions.insert(1, add_priority_fee);
        
                    let recent_blockhash = blocking_client.get_latest_blockhash()?;
                    let tx = Transaction::new_signed_with_payer(
                        &instructions,
                        Some(&keypair_arc.pubkey()),
                        &vec![&*keypair_arc],
                        recent_blockhash,
                    );
                    println!("Debugging step7... {:?}", instructions);
                    let sig_result = task::spawn_blocking(move || {
                        common::rpc::send_txn(&blocking_client, &tx, true) // Wrap send_txn in spawn_blocking
                    }).await?; // Await the result and handle errors

                    match sig_result {
                        Ok(signature) => {
                            println!("Transaction sent successfully: {:?}", signature);
                            Ok(vec![signature.to_string()])
                        }
                        Err(error) => {
                            println!("Transaction failed: {}", error);
                            Err(anyhow::Error::from(error)) // Propagate the error
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Error unpacking user input token account: {:?}", e);
                    return Err(anyhow!("Failed to unpack user input token account: {}", e));
                }
            }
            
        }
        Ok(Err(_err)) => {
            // Handle the error from Program::account
            let instructions = Vec::new();
            new_signed_and_send(&blocking_client, &keypair_arc, instructions, use_jito).await
        }
        Err(_err) => {
            // Handle the error from spawn_blocking
            let instructions = Vec::new();
            new_signed_and_send(&blocking_client, &keypair_arc, instructions, use_jito).await
        }
    }

    
}
