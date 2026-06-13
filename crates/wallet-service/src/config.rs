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
