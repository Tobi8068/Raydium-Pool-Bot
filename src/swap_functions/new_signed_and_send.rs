use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction,
    signer::{keypair::Keypair, Signer},
    transaction::Transaction
};
use anyhow::{anyhow, Result};
use std::env;
use tracing::info;
use std::str::FromStr;

pub async fn new_signed_and_send(
    client: &RpcClient,
    keypair: &Keypair,
    instructions: Vec<Instruction>,
    use_jito: bool,
) -> Result<Vec<String>> {
    // let unit_limit = get_unit_limit();
    // let unit_price = get_unit_price();
    // // If not using Jito, manually set the compute unit price and limit
    // if !use_jito {
    //     let modify_compute_units =
    //         solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(
    //             unit_limit,
    //         );
    //     let add_priority_fee =
    //         solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(
    //             unit_price,
    //         );
    //     instructions.insert(0, modify_compute_units);
    //     instructions.insert(1, add_priority_fee);
    // }
    // send init tx
    let recent_blockhash = client.get_latest_blockhash()?;
    let txn = Transaction::new_signed_with_payer(
        &instructions,
        Some(&keypair.pubkey()),
        &vec![&*keypair],
        recent_blockhash,
    );

    if env::var("TX_SIMULATE").ok() == Some("true".to_string()) {
        let simulate_result = client.simulate_transaction(&txn)?;
        if let Some(logs) = simulate_result.value.logs {
            for log in logs {
                info!("{}", log);
            }
        }
        return match simulate_result.value.err {
            Some(err) => Err(anyhow!("{}", err)),
            None => Ok(vec![]),
        };
    }

    if use_jito {
        // jito implementation placeholder
        Ok(vec![])
    } else {
        let sig = common::rpc::send_txn(&client, &txn, true)?;
        info!("signature: {:?}", sig);
        Ok(vec![sig.to_string()])
    }
}

pub fn get_unit_price() -> u64 {
    env::var("UNIT_PRICE")
        .ok()
        .and_then(|v| u64::from_str(&v).ok())
        .unwrap_or(20000)
}

// pub fn get_unit_limit() -> u32 {
//     env::var("UNIT_LIMIT")
//         .ok()
//         .and_then(|v| u32::from_str(&v).ok())
//         .unwrap_or(200_000)
// }