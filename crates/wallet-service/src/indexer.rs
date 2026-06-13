//! Indexer: exports decrypted memos from the wallet's sqlite store into Postgres.
//!
//! The wallet db is the chain-derived source of truth (compact blocks were trial-
//! decrypted with the org viewing key during sync). This module reads received
//! outputs + memos from it over a *read-only* sqlite connection and reconstructs
//! the audit tables. Runs as a periodic task, fully idempotent: every pass
//! upserts, so `TRUNCATE` + next pass = "rebuild from chain".

use anyhow::{anyhow, Context, Result};
use deadpool_postgres::Pool;
use time::OffsetDateTime;

use memo_schema::Record;

const EXPORT_SQL: &str = "
    SELECT t.txid,
           ro.pool,
           ro.output_index,
           t.mined_height,
           b.time,
           a.address,
           acct.hd_account_index,
           ro.memo
    FROM v_received_outputs ro
    JOIN transactions t ON t.id_tx = ro.transaction_id
    JOIN accounts acct ON acct.id = ro.account_id
    LEFT JOIN addresses a ON a.id = ro.address_id
    LEFT JOIN blocks b ON b.height = t.mined_height
    WHERE ro.memo IS NOT NULL
      AND t.mined_height IS NOT NULL
";

struct ExportRow {
    txid: String,
    pool: i64,
    output_index: i64,
    height: i64,
    block_time: Option<OffsetDateTime>,
    address: Option<String>,
    user_index: Option<i64>,
    memo: Vec<u8>,
}

/// One indexer pass: read all mined memo outputs from the wallet db, decode,
/// and upsert into Postgres. Also marks submissions confirmed by txid.
pub async fn run_once(wallet_db_path: &str, pg: &Pool) -> Result<usize> {
    let rows = read_wallet_outputs(wallet_db_path)?;
    if rows.is_empty() {
        return Ok(0);
    }

    let client = pg.get().await.context("pg pool")?;
    let mut written = 0usize;

    for row in &rows {
        let record = match memo_schema::decode_memo(&row.memo) {
            Ok(r) => r,
            // Not ours / change memo / corrupt: skip quietly.
            Err(_) => continue,
        };

        match record {
            Record::Enroll(e) => {
                let address = row.address.clone().unwrap_or_default();
                client
                    .execute(
                        "INSERT INTO address_book (address, user_index, name, role, active, txid, block_height, block_time)
                         VALUES ($1, $2, $3, $4, TRUE, $5, $6, $7)
                         ON CONFLICT (address) DO UPDATE
                           SET name = EXCLUDED.name, role = EXCLUDED.role,
                               txid = EXCLUDED.txid, block_height = EXCLUDED.block_height,
                               block_time = EXCLUDED.block_time",
                        &[
                            &address,
                            &row.user_index.map(|i| i as i32),
                            &e.name,
                            &e.role,
                            &row.txid,
                            &row.height,
                            &row.block_time,
                        ],
                    )
                    .await
                    .context("upsert address_book")?;
                written += 1;
            }
            Record::Event(e) => {
                let client_ts = OffsetDateTime::from_unix_timestamp(e.client_ts as i64)
                    .unwrap_or(OffsetDateTime::UNIX_EPOCH);
                client
                    .execute(
                        "INSERT INTO audit_records
                           (txid, output_pool, output_index, block_height, block_time,
                            address, user_index, item_id, event_type, quantity, temp_centi,
                            client_ts, notes)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                         ON CONFLICT (txid, output_pool, output_index) DO NOTHING",
                        &[
                            &row.txid,
                            &pool_name(row.pool),
                            &(row.output_index as i32),
                            &row.height,
                            &row.block_time,
                            &row.address,
                            &row.user_index.map(|i| i as i32),
                            &e.item_id,
                            &e.event_type.as_str(),
                            &(e.quantity as i64),
                            &e.temp_centi,
                            &client_ts,
                            &e.notes,
                        ],
                    )
                    .await
                    .context("insert audit_records")?;
                written += 1;
            }
        }

        // Any submission whose tx is now mined is confirmed.
        client
            .execute(
                "UPDATE submissions SET status = 'confirmed', updated_at = now()
                 WHERE txid = $1 AND status <> 'confirmed'",
                &[&row.txid],
            )
            .await
            .context("confirm submissions")?;
    }

    Ok(written)
}

fn read_wallet_outputs(wallet_db_path: &str) -> Result<Vec<ExportRow>> {
    let conn = rusqlite::Connection::open_with_flags(
        wallet_db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .context("open wallet db read-only")?;

    let mut stmt = conn.prepare(EXPORT_SQL).context("prepare export query")?;
    let rows = stmt
        .query_map([], |r| {
            let txid_blob: Vec<u8> = r.get(0)?;
            // txid display order is byte-reversed from the internal encoding.
            let txid_hex = hex::encode(txid_blob.iter().rev().cloned().collect::<Vec<u8>>());
            let unix_time: Option<i64> = r.get(4)?;
            Ok(ExportRow {
                txid: txid_hex,
                pool: r.get(1)?,
                output_index: r.get(2)?,
                height: r.get(3)?,
                block_time: unix_time
                    .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok()),
                address: r.get(5)?,
                user_index: r.get(6)?,
                memo: r.get(7)?,
            })
        })
        .context("query export rows")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("read export rows: {e}"))?;

    Ok(rows)
}

fn pool_name(pool: i64) -> &'static str {
    match pool {
        2 => "sapling",
        3 => "orchard",
        _ => "unknown",
    }
}
