use crate::jito::{TipPercentileData, TIPS_PERCENTILE};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use std::env;

pub async fn _tip_stream() -> Result<()> {
    let tip_stream_url = env::var("JITO_TIP_STREAM_URL").context("JITO_TIP_STREAM_URL environment variable not set")?;
    let (ws_stream, _) = connect_async(tip_stream_url.to_string())
        .await
        .context("Failed to connect to WebSocket server")?;

    info!("Connected to WebSocket server: tip_stream");

    let (mut _write, mut read) = ws_stream.split();

    while let Some(message) = read.next().await {
        match message {
            Ok(Message::Text(text)) => {
                debug!("Received text message: {}", text);

                match serde_json::from_str::<Vec<TipPercentileData>>(&text) {
                    Ok(data) => {
                        if !data.is_empty() {
                            *TIPS_PERCENTILE.write().await = data.first().cloned();
                        } else {
                            warn!("Received an empty data.")
                        }
                    }
                    Err(e) => {
                        error!("Failed to deserialize JSON: {:?}", e);
                    }
                }
            }
            Ok(Message::Close(close)) => {
                info!("Connection closed: {:?}", close);
                break;
            }
            Err(e) => {
                error!("Error receiving message: {:?}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
