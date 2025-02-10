use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use dotenv::dotenv;
use serde::Deserialize;
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signer::{keypair::{self, Keypair}, Signer},
};
use spl_associated_token_account::get_associated_token_address;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info};

mod swap_functions;
use swap_functions::swap_amm::*;
use swap_functions::swap_cpmm::*;
use swap_functions::swap_clmm::swap_clmm;

mod jito;

#[derive(ValueEnum, Debug, Clone, Deserialize)]
pub enum PoolType {
    #[serde(rename = "amm")]
    AMM,
    #[serde(rename = "cpmm")]
    CPMM,
    #[serde(rename = "clmm")]
    CLMM,
}

pub fn get_wallet() -> Result<Keypair> {
    let wallet = Keypair::from_base58_string(&env::var("PRIVATE_KEY")?);
    return Ok(wallet);
}

async fn get_pool_type(client: &RpcClient, pool_id: &str) -> Result<PoolType> {
    let pool_pubkey = Pubkey::from_str(pool_id)?;
    let account = client.get_account(&pool_pubkey)?;

    // Check program ID to determine pool type
    match account.owner.to_string().as_str() {
        "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8" => Ok(PoolType::AMM), // Raydium AMM
        "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK" => Ok(PoolType::CLMM), // Orca Whirlpools
        "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" => Ok(PoolType::CPMM), // Orca CPMM
        _ => Err(anyhow!(
            "Unknown pool type for program ID: {}",
            account.owner
        )),
    }
}

async fn swap(
    pool_id: Option<&str>,
    keypair: Keypair,
    mint_str: &str,
    amount_in: f64,
    swap_direction: SwapDirection,
    in_type: SwapInType,
    slippage: u64,
    use_jito: bool,
    pool_type: PoolType,
) -> Result<Vec<String>> {
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    // Create both blocking and non-blocking clients
    let blocking_client = Arc::new(RpcClient::new(rpc_url.clone()));
    let nonblocking_client = Arc::new(NonblockingRpcClient::new(rpc_url));

    match pool_type {
        PoolType::AMM => {
            swap_amm(
                pool_id,
                keypair,
                mint_str,
                amount_in,
                swap_direction,
                in_type,
                slippage,
                use_jito,
                blocking_client,
                nonblocking_client,
            )
            .await
        }
        PoolType::CPMM => {
            swap_cpmm(
                pool_id,
                keypair,
                mint_str,
                slippage,
                use_jito,
                blocking_client,
                nonblocking_client,
            )
            .await
        }
        PoolType::CLMM => {
            swap_clmm(
                pool_id,
                keypair,
                mint_str,
                slippage,
                use_jito,
                blocking_client,
                nonblocking_client,
            )
            .await
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let pool_id =
        env::var("TARGET_ADDRESS").context("TARGET_ADDRESS environment variable not set")?;
    let target_price_str =
        env::var("TARGET_PRICE").context("TARGET_ADDRESS environment variable not set")?;
    let target_price: f64 = target_price_str  
        .parse::<f64>()  
        .context("Failed to parse TARGET_PRICE as f64")?; 
    let swap_amount = 1.0; // 100% of tokens
    let rpc_url = env::var("RPC_URL").expect("RPC_URL environment variable not set");
    let rpc_client = Arc::new(RpcClient::new(rpc_url));
    let pool_type = get_pool_type(&rpc_client, &pool_id).await?;

    
    println!("{:?}", pool_type);
    
    let wallet = get_wallet()?;
    let mint = env::var("MINT_ADDRESS")?;

    let use_jito = true;

    let mut is_swap = false;
    let mut is_stop = false;

    if use_jito {
        jito::init_tip_accounts()
            .await
            .map_err(|err| {
                info!("failed to get tip accounts: {:?}", err);
                err
            })
            .unwrap();
        jito::init_tip_amounts()
            .await
            .map_err(|err| {
                info!("failed to init tip amounts: {:?}", err);
                err
            })
            .unwrap();
    }

    loop {
        if !is_stop {
            match pool_type {
                PoolType::AMM => {
                    println!("Get Pool Price AMM ...");
                    match get_pool_price(Some(&pool_id), None).await {
                        Ok((_base_amount, _quote_amount, current_price)) => {
                            println!("Current Price {} SOL", current_price);
                            if current_price > target_price {
                                is_swap = true;
                            }
                        }
                        Err(e) => eprintln!("Error fetching pool price: {}", e),
                    }
                }
                PoolType::CPMM => {
                    println!("Get Pool Price CPMM ...");
                    match get_pool_price_cpmm(Some(&pool_id), wallet.insecure_clone(), rpc_client.clone()).await {
                        Ok(current_price) => {
                            println!("Current Price {} SOL", current_price);
                            if current_price > target_price {
                                is_swap = true;
                            }
                        }
                        Err(e) => eprintln!("Error fetching pool price: {}", e),
                    }
                }
                PoolType::CLMM => {
                    println!("Get Pool Price CLMM ...");
                    is_swap = true;
                }
            }
            if is_swap {
                match swap(
                    Some(&pool_id),
                    wallet.insecure_clone(),
                    &mint,
                    swap_amount,
                    SwapDirection::Sell,
                    SwapInType::Pct,
                    30,
                    use_jito,
                    pool_type.clone(),
                )
                .await
                {
                    Ok(_signatures) => {
                        is_stop = false;
        
                        // Optional: Verify the token balance is now 0
                        let token_ata =
                            get_associated_token_address(&wallet.pubkey(), &Pubkey::from_str(&mint)?);
                        match rpc_client.get_token_account_balance(&token_ata) {
                            Ok(balance) => {
                                if balance.amount == "0" {
                                    println!("Swap completed successfully! Token balance is now 0");
                                    break; // Exit the monitoring loop
                                } else {
                                    println!(
                                        "Warning: Token balance is not 0 after swap: {}",
                                        balance.amount
                                    );
                                }
                            }
                            Err(e) => {
                                error!("Failed to check final token balance: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to initiate swap: {}", e);
                    }
                }
            }
        } else {
            // println!("Current pool price is lower than target price");
        }
    }
    Ok(())
}
