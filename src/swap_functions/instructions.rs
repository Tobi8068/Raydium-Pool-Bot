use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::Keypair,
};
use anyhow::Result;
use std::{env, rc::Rc, sync::Arc, str::FromStr};
use anchor_client::{Client, Cluster};
use raydium_cp_swap::AUTH_SEED;
use raydium_cp_swap::accounts as raydium_cp_accounts;
use raydium_cp_swap::instruction as raydium_cp_instructions;

pub fn create_ata_token_account_instr(
    keypair: Keypair,
    token_program: Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Vec<Instruction>> {
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let url = Cluster::Custom(rpc_url, ws_url.to_string());
    // Client.
    let keypair_arc = Arc::new(keypair);
    let client = Client::new(url, keypair_arc.clone());
    let program = client.program(token_program)?;
    let instructions = program
        .request()
        .instruction(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &program.payer(),
                owner,
                mint,
                &token_program,
            ),
        )
        .instructions()?;
    Ok(instructions)
}

pub fn swap_base_output_instr(
    keypair: Keypair,
    pool_id: Pubkey,
    amm_config: Pubkey,
    observation_account: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    input_token_mint: Pubkey,
    output_token_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    max_amount_in: u64,
    amount_out: u64,
) -> Result<Vec<Instruction>> {
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let ws_url = "wss://api.mainnet-beta.solana.com/";
    let url = Cluster::Custom(rpc_url, ws_url.to_string());
    // Client.
    let client = Client::new(url, Rc::new(keypair));
    let program_id_str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
    let program_id = Pubkey::from_str(program_id_str)?;
    let program = client.program(program_id)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let instructions = program
        .request()
        .accounts(raydium_cp_accounts::Swap {
            payer: program.payer(),
            authority,
            amm_config,
            pool_state: pool_id,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            input_token_program,
            output_token_program,
            input_token_mint,
            output_token_mint,
            observation_state: observation_account,
        })
        .args(raydium_cp_instructions::SwapBaseOutput {
            max_amount_in,
            amount_out,
        })
        .instructions()?;
    Ok(instructions)
}
