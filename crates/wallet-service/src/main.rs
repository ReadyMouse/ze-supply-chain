// Wallet Service — Entry Point
//
//   CLI and server bootstrap for the Zcash signer + indexer. Supports gen-seed
//   subcommand and default serve mode with periodic indexer ticks.
//
// INPUT:
//   - CLI args (gen-seed | serve)
//   - Environment via config::Config::from_env()
//
// OUTPUT:
//   - Running HTTP server on WALLET_SERVICE_ADDR (default 7001)
//   - Printed BIP-39 seed when gen-seed is invoked
//
// NOTES:
//   Indexer runs every 15s in a background task. Requires proprietary org seed.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

mod api;
mod block_cache;
mod config;
mod indexer;
mod pg;
mod wallet;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser)]
#[command(about = "Zcash audit-log wallet service (signer + indexer)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Generate a fresh 24-word seed phrase and exit.
    GenSeed,
    /// Run the service (default).
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wallet_service=debug".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::GenSeed) => {
            let mnemonic = bip39::Mnemonic::generate(24).context("generate mnemonic")?;
            println!("{mnemonic}");
            println!("\nStore this in .env as WALLET_SEED_PHRASE — it controls real funds.");
            Ok(())
        }
        _ => serve().await,
    }
}

async fn serve() -> Result<()> {
    let cfg = config::Config::from_env()?;

    let pg = pg::connect(&cfg.database_url).await?;
    pg::apply_schema(&pg).await?;
    tracing::info!("postgres ready");

    let wallet = wallet::WalletActor::spawn(cfg.clone()).await?;
    tracing::info!("wallet actor running");

    // Periodic indexer pass: wallet sqlite -> postgres.
    {
        let pg = pg.clone();
        let path = cfg.wallet_db_path.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                match indexer::run_once(&path, &pg).await {
                    Ok(n) if n > 0 => tracing::info!("indexer: wrote {n} records"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("indexer pass failed: {e:#}"),
                }
            }
        });
    }

    let state = api::AppState {
        wallet,
        pg,
        wallet_db_path: Arc::new(cfg.wallet_db_path.clone()),
    };
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr)
        .await
        .with_context(|| format!("bind {}", cfg.listen_addr))?;
    tracing::info!("wallet-service listening on {}", cfg.listen_addr);
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
