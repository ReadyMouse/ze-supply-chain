// Wallet Service Configuration
//
//   Loads runtime settings from environment variables: seed, birthday, Postgres,
//   lightwalletd URL, batching policy, and listen address.
//
// INPUT:
//   - WALLET_SEED_PHRASE, WALLET_BIRTHDAY, DATABASE_URL, LIGHTWALLETD_URL,
//     WALLET_DB_PATH, WALLET_SERVICE_ADDR, BATCH_MAX_RECORDS, BATCH_MAX_AGE_SECS
//
// OUTPUT:
//   - Config struct consumed by wallet actor and HTTP API
//
// NOTES:
//   Missing required env vars fail fast via anyhow context. Batch defaults: 5
//   records or 120 seconds.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

use anyhow::{Context, Result};

#[derive(Clone)]
pub struct Config {
    pub seed_phrase: String,
    pub birthday: u32,
    pub database_url: String,
    pub lightwalletd_url: String,
    pub wallet_db_path: String,
    pub listen_addr: String,
    pub batch_max_records: usize,
    pub batch_max_age_secs: u64,
}

fn var(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("missing env var {name}"))
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Config {
            seed_phrase: var("WALLET_SEED_PHRASE")?,
            birthday: var("WALLET_BIRTHDAY")?.parse().context("WALLET_BIRTHDAY")?,
            database_url: var("DATABASE_URL")?,
            lightwalletd_url: var("LIGHTWALLETD_URL")?,
            wallet_db_path: var("WALLET_DB_PATH")?,
            listen_addr: std::env::var("WALLET_SERVICE_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:7001".into()),
            batch_max_records: std::env::var("BATCH_MAX_RECORDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            batch_max_age_secs: std::env::var("BATCH_MAX_AGE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
        })
    }
}
