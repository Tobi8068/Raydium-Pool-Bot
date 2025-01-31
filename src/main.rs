use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tokio;
use std::sync::Arc;
use raydium_amm::state::{Loadable, AmmInfo};
use anyhow::{anyhow, Context, Result};
use common::common_utils;
use spl_token_2022::amount_to_ui_amount;
use tracing::debug;
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use serde::Deserialize;
use std::env;
use reqwest::Proxy;
use dotenv::dotenv;

#[derive(Debug, Deserialize, Clone)]
pub struct Pool {
    pub id: String,
    #[serde(rename = "programId")]
    pub program_id: String,
    #[serde(rename = "mintA")]
    pub mint_a: Mint,
    #[serde(rename = "mintB")]
    pub mint_b: Mint,
    #[serde(rename = "marketId")]
    pub market_id: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Mint {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
}


#[derive(Debug, Deserialize)]
pub struct PoolData {
    pub data: Vec<Pool>,
}

impl PoolData {
    pub fn get_pool(&self) -> Option<Pool> {
        self.data.first().cloned()
    }
}

#[derive(Debug, Deserialize)]
pub struct PoolInfo {
    pub success: bool,
    pub data: PoolData,
}

pub const AMM_PROGRAM: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

async fn get_pool_state_by_mint(
    rpc_client: Arc<solana_client::rpc_client::RpcClient>,
    mint: &str,
) -> Result<(Pubkey, AmmInfo)> {
    debug!("finding pool state by mint: {}", mint);
    // (pc_mint, coin_mint)
    let pairs = vec![
        // pump pool
        (
            Some(spl_token::native_mint::ID),
            Pubkey::from_str(mint).ok(),
        ),
        // general pool
        (
            Pubkey::from_str(mint).ok(),
            Some(spl_token::native_mint::ID),
        ),
    ];

    let pool_len = core::mem::size_of::<raydium_amm::state::AmmInfo>() as u64;
    let amm_program = Pubkey::from_str(AMM_PROGRAM)?;
    // Find matching AMM pool from mint pairs by filter
    let mut found_pools = None;
    for (coin_mint, pc_mint) in pairs {
        debug!(
            "get_pool_state_by_mint filter: coin_mint: {:?}, pc_mint: {:?}",
            coin_mint, pc_mint
        );
        let filters = match (coin_mint, pc_mint) {
            (None, None) => Some(vec![RpcFilterType::DataSize(pool_len)]),
            (Some(coin_mint), None) => Some(vec![
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(400, &coin_mint.to_bytes())),
                RpcFilterType::DataSize(pool_len),
            ]),
            (None, Some(pc_mint)) => Some(vec![
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(432, &pc_mint.to_bytes())),
                RpcFilterType::DataSize(pool_len),
            ]),
            (Some(coin_mint), Some(pc_mint)) => Some(vec![
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(400, &coin_mint.to_bytes())),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(432, &pc_mint.to_bytes())),
                RpcFilterType::DataSize(pool_len),
            ]),
        };
        let pools =
            common::rpc::get_program_accounts_with_filters(&rpc_client, amm_program, filters)
                .unwrap();
        if !pools.is_empty() {
            found_pools = Some(pools);
            break;
        }
    }

    match found_pools {
        Some(pools) => {
            let pool = &pools[0];
            let pool_state = raydium_amm::state::AmmInfo::load_from_bytes(&pools[0].1.data)?;
            Ok((pool.0, pool_state.clone()))
        }
        None => {
            return Err(anyhow!("NotFoundPool: pool state not found"));
        }
    }
}

async fn get_pool_info(mint1: &str, mint2: &str) -> Result<PoolData> {
    let mut client_builder = reqwest::Client::builder();
    if let Ok(http_proxy) = env::var("HTTP_PROXY") {
        let proxy = Proxy::all(http_proxy)?;
        client_builder = client_builder.proxy(proxy);
    }
    let client = client_builder.build()?;

    let result = client
        .get("https://api-v3.raydium.io/pools/info/mint")
        .query(&[
            ("mint1", mint1),
            ("mint2", mint2),
            ("poolType", "standard"),
            ("poolSortField", "default"),
            ("sortType", "desc"),
            ("pageSize", "1"),
            ("page", "1"),
        ])
        .send()
        .await?
        .json::<PoolInfo>()
        .await
        .context("Failed to parse pool info JSON")?;
    Ok(result.data)
}

async fn get_pool_state(
  rpc_client: Arc<solana_client::rpc_client::RpcClient>,
  pool_id: Option<&str>,
  mint: Option<&str>,
) -> Result<(Pubkey, AmmInfo)> {
  if let Some(pool_id) = pool_id {
      debug!("finding pool state by pool_id: {}", pool_id);
      let amm_pool_id = Pubkey::from_str(pool_id)?;
      let account_data = common::rpc::get_account(&rpc_client, &amm_pool_id)?
          .ok_or(anyhow!("NotFoundPool: pool state not found"))?;
      let pool_state = AmmInfo::load_from_bytes(&account_data)?.clone();
      Ok((amm_pool_id, pool_state))
  } else {
      if let Some(mint) = mint {
          // find pool by mint via rpc
          if let Ok(pool_state) = get_pool_state_by_mint(rpc_client.clone(), mint).await {
              return Ok(pool_state);
          }
          // find pool by mint via raydium api
          let pool_data = get_pool_info(&spl_token::native_mint::ID.to_string(), mint).await;
          if let Ok(pool_data) = pool_data {
              let pool = pool_data
                  .get_pool()
                  .ok_or(anyhow!("NotFoundPool: pool not found in raydium api"))?;
              let amm_pool_id = Pubkey::from_str(&pool.id)?;
              debug!("finding pool state by raydium api: {}", amm_pool_id);
              let account_data = common::rpc::get_account(&rpc_client, &amm_pool_id)?
                  .ok_or(anyhow!("NotFoundPool: pool state not found"))?;
              let pool_state = AmmInfo::load_from_bytes(&account_data)?.clone();
              return Ok((amm_pool_id, pool_state));
          }
          Err(anyhow!("NotFoundPool: pool state not found"))
      } else {
          Err(anyhow!("NotFoundPool: pool state not found"))
      }
  }
}

async fn get_pool_price(
  pool_id: Option<&str>,
  mint: Option<&str>,
) -> Result<(f64, f64, f64)> {
    let rpc_url = env::var("RPC_URL")
    .context("RPC_URL environment variable not set")?;
  let rpc_client: RpcClient =
      RpcClient::new(rpc_url.to_string());
  let client: Arc<RpcClient> = Arc::new(rpc_client);

  let (amm_pool_id, pool_state) = get_pool_state(client.clone(), pool_id, mint).await?;

  // debug!("pool_state : {:#?}", pool_state);

  let load_pubkeys = vec![pool_state.pc_vault, pool_state.coin_vault];
  let rsps = common::rpc::get_multiple_accounts(&client, &load_pubkeys).unwrap();

  let amm_pc_vault_account = rsps[0].clone();
  let amm_coin_vault_account = rsps[1].clone();

  let amm_pc_vault =
      common_utils::unpack_token(&amm_pc_vault_account.as_ref().unwrap().data).unwrap();
  let amm_coin_vault =
      common_utils::unpack_token(&amm_coin_vault_account.as_ref().unwrap().data).unwrap();

  let (base_account, quote_account) = if amm_coin_vault.base.is_native() {
      (
          (
              pool_state.pc_vault_mint,
              amount_to_ui_amount(amm_pc_vault.base.amount, pool_state.pc_decimals as u8),
          ),
          (
              pool_state.coin_vault_mint,
              amount_to_ui_amount(amm_coin_vault.base.amount, pool_state.coin_decimals as u8),
          ),
      )
  } else {
      (
          (
              pool_state.coin_vault_mint,
              amount_to_ui_amount(amm_coin_vault.base.amount, pool_state.coin_decimals as u8),
          ),
          (
              pool_state.pc_vault_mint,
              amount_to_ui_amount(amm_pc_vault.base.amount, pool_state.pc_decimals as u8),
          ),
      )
  };

  let price = quote_account.1 / base_account.1;

  debug!(
      "calculate pool[{}]: {}: {}, {}: {}, price: {} sol",
      amm_pool_id, base_account.0, base_account.1, quote_account.0, quote_account.1, price
  );

  Ok((base_account.1, quote_account.1, price))
}


#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let pool_id = env::var("TARGET_ADDRESS").context("TARGET_ADDRESS environment variable not set")?;
    match get_pool_price(Some(&pool_id), None).await {
        Ok(result) => println!("Account Info: {:?}", result),
        Err(e) => eprintln!("Error fetching account info: {}", e),
    }
    Ok(())
}
