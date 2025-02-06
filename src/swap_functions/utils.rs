use anchor_lang::AccountDeserialize;
use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    account::Account
};
use spl_token_2022::extension::{
    transfer_fee::{TransferFeeConfig, MAX_FEE_BASIS_POINTS},
    BaseState, BaseStateWithExtensions, StateWithExtensionsMut,
};
use std::ops::Mul;

use borsh::BorshDeserialize;
use borsh_derive::{BorshSerialize, BorshDeserialize};
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct PoolState {
    pub trade_fee_rate: u64,
    pub protocol_fee_rate: u64,
    pub fund_fee_rate: u64,
    // Other fields...
}

pub fn deserialize_anchor_account<T: AccountDeserialize>(account: &Account) -> Result<T> {
    let mut data: &[u8] = &account.data;
    T::try_deserialize(&mut data).map_err(Into::into)
}

/// Calculate the fee for output amount
pub fn get_transfer_inverse_fee<'data, S: BaseState>(
    account_state: &StateWithExtensionsMut<'data, S>,
    epoch: u64,
    post_fee_amount: u64,
) -> u64 {
    let fee = if let Ok(transfer_fee_config) = account_state.get_extension::<TransferFeeConfig>() {
        let transfer_fee = transfer_fee_config.get_epoch_fee(epoch);
        if u16::from(transfer_fee.transfer_fee_basis_points) == MAX_FEE_BASIS_POINTS {
            u64::from(transfer_fee.maximum_fee)
        } else {
            transfer_fee_config
                .calculate_inverse_epoch_fee(epoch, post_fee_amount)
                .unwrap()
        }
    } else {
        0
    };
    fee
}

pub fn amount_with_slippage(amount: u64, slippage: f64, round_up: bool) -> u64 {
    if round_up {
        (amount as f64).mul(1_f64 + slippage).ceil() as u64
    } else {
        (amount as f64).mul(1_f64 - slippage).floor() as u64
    }
}

pub fn fetch_pool_state(rpc_client: &RpcClient, pool_id: &Pubkey) -> Result<PoolState> {
    let account_data = rpc_client.get_account_data(pool_id)?;
    let pool_state = PoolState::try_from_slice(&account_data)?;
    Ok(pool_state)
}

pub fn get_fee_rates(rpc_client: &RpcClient, pool_id: &Pubkey) -> Result<(u64, u64, u64)> {
    let pool_state = fetch_pool_state(rpc_client, pool_id)?;
    Ok((pool_state.trade_fee_rate, pool_state.protocol_fee_rate, pool_state.fund_fee_rate))
}

pub fn calculate_amount_out_less_fee(
    amount_in: u64,
    trade_fee_rate: u64,
    protocol_fee_rate: u64,
    fund_fee_rate: u64,
) -> u64 {
    // Calculate the total fee rate
    let total_fee_rate = trade_fee_rate + protocol_fee_rate + fund_fee_rate;

    // Calculate the total fees
    let total_fees = amount_in * total_fee_rate / 1_000_000; // Assuming fee rates are in millionths

    // Calculate the amount out less fee
    let amount_out_less_fee = amount_in - total_fees;

    amount_out_less_fee
}