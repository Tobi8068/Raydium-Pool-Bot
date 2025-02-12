# Solana Raydium Pool Tracking Trading Bot 🌊🚀  

Welcome to the **Solana Raydium Pool Tracking Trading Bot**! This is a Rust-based trading bot designed specifically for monitoring and trading on Raydium pools within the Solana blockchain. The bot continuously tracks market prices and executes trades based on specified target prices that you can set in the configuration.  

## Features ✨  

- Monitors prices from Raydium Automated Market Maker (AMM) pools. 📊  
- Supports liquidity pool price tracking for different models: CLMM (Constant Product Market Maker) and CPMM (Constant Mean Market Maker). 🔄  
- Executes trades when the pool price exceeds your predefined target price. 💰  
- Highly configurable via a **.env** file. ⚙️  

## Requirements 📋  

- **Rust Version**: The bot is developed with Rust 1.84. Ensure you have this version or later installed on your machine to run the bot smoothly.  

## Configuration ⚙️  

To get started, create a `.env` file in the root directory of the project and populate it with the following variables:

```plaintext  
TARGET_PRICE=0.000000000000001  
TARGET_ADDRESS=  
MINT_ADDRESS=  
RPC_URL=  
PRIVATE_KEY=  
SLIPPAGE=  
JITO_BLOCK_ENGINE_URL=  
JITO_TIP_STREAM_URL=  
JITO_TIP_PERCENTILE=  
JITO_TIP_VALUE=  
```

### Parameter Details

- **TARGET_PRICE**: The price at which you want the bot to execute trades. 💵
- **TARGET_ADDRESS**: The pool address where you want to execute the trades. 📍
- **MINT_ADDRESS**: The token mint address for the asset you are trading. 🪙
- **RPC_URL**: The URL for the Solana JSON RPC endpoint. 🌐
- **PRIVATE_KEY**: Your private key for wallet access (ensure this is kept secure). 🔑
- **SLIPPAGE**: The maximum acceptable slippage for trades. 📉
- **JITO_BLOCK_ENGINE_URL**: URL for Jito block engine integration. 🔄
- **JITO_TIP_STREAM_URL**: URL for receiving tips via Jito stream. 💸
- **JITO_TIP_PERCENTILE**: Desired percentile for tips. 🎯
- **JITO_TIP_VALUE**: Value of the tip to be used. 💡

## Usage 🚀

1. Clone the repository to your local machine. 📥
2. Ensure that you have Rust 1.84 installed. 🛠️
3. Configure the `.env` file as described above. 📜
4. Build and run the bot using Cargo:

   ```bash
   cargo build
   cargo run
   ```

5. The bot will start monitoring the prices and execute trades based on your configuration. 🚦

## Contributing 🌟  

If you are interested in this repository, feel free to [fork it](https://github.com/yourusername/yourrepository/fork) and give me [a big star! ⭐](https://github.com/tobi8068/Raydium-Pool-Bot/stargazers) Your support is much appreciated and helps improve the project further. 🚀  

Thank you for considering contributing to this project! 💖

