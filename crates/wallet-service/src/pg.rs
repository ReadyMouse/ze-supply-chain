use anyhow::{Context, Result};
use deadpool_postgres::{Manager, Pool};
use tokio_postgres::NoTls;

pub async fn connect(database_url: &str) -> Result<Pool> {
    let config: tokio_postgres::Config = database_url.parse().context("parse DATABASE_URL")?;
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager)
        .max_size(8)
        .build()
        .context("build pg pool")?;
    // Fail fast on a bad connection string.
    let _ = pool.get().await.context("connect to postgres")?;
    Ok(pool)
}

pub async fn apply_schema(pool: &Pool) -> Result<()> {
    let schema = include_str!("../../../migrations/schema.sql");
    let client = pool.get().await?;
    client.batch_execute(schema).await.context("apply schema")?;
    Ok(())
}
