//! Internal HTTP API consumed by the gateway. Not exposed publicly.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use uuid::Uuid;

use memo_schema::Record;

use crate::indexer;
use crate::wallet::{QueuedRecord, WalletHandle};

#[derive(Clone)]
pub struct AppState {
    pub wallet: WalletHandle,
    pub pg: Pool,
    pub wallet_db_path: Arc<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/enroll", post(enroll))
        .route("/submit", post(submit))
        .route("/process-batch", post(process_batch))
        .route("/split-notes", post(split_notes))
        .route("/rebuild", post(rebuild))
        .route("/status", get(status))
        .with_state(state)
}

type ApiError = (StatusCode, String);

fn internal(e: anyhow::Error) -> ApiError {
    tracing::error!("api error: {e:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
}

#[derive(serde::Deserialize)]
struct EnrollReq {
    user_index: u32,
    name: String,
    role: String,
    submission_id: Uuid,
}

#[derive(serde::Serialize)]
struct EnrollResp {
    address: String,
}

async fn enroll(
    State(state): State<AppState>,
    Json(req): Json<EnrollReq>,
) -> Result<Json<EnrollResp>, ApiError> {
    let address = state
        .wallet
        .ensure_account(req.user_index)
        .await
        .map_err(internal)?;

    let record = Record::Enroll(memo_schema::EnrollRecord {
        name: req.name,
        role: req.role,
    });
    memo_schema::encode_memo(&record)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    state
        .wallet
        .enqueue(QueuedRecord {
            submission_id: req.submission_id,
            user_index: req.user_index,
            record,
        })
        .await
        .map_err(internal)?;

    Ok(Json(EnrollResp { address }))
}

#[derive(serde::Deserialize)]
struct SubmitReq {
    submission_id: Uuid,
    user_index: u32,
    record: Record,
}

async fn submit(
    State(state): State<AppState>,
    Json(req): Json<SubmitReq>,
) -> Result<StatusCode, ApiError> {
    // Validate the record fits in a memo before accepting it.
    memo_schema::encode_memo(&req.record)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    state
        .wallet
        .enqueue(QueuedRecord {
            submission_id: req.submission_id,
            user_index: req.user_index,
            record: req.record,
        })
        .await
        .map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(serde::Serialize)]
struct ProcessBatchResp {
    broadcast: Vec<BroadcastEntry>,
}

#[derive(serde::Serialize)]
struct BroadcastEntry {
    submission_id: Uuid,
    txid: String,
}

async fn process_batch(
    State(state): State<AppState>,
) -> Result<Json<ProcessBatchResp>, ApiError> {
    let sent = state.wallet.process_batch().await.map_err(internal)?;

    // Mark submissions broadcast with their txid.
    if !sent.is_empty() {
        let client = state.pg.get().await.map_err(|e| internal(e.into()))?;
        for (submission_id, txid) in &sent {
            client
                .execute(
                    "UPDATE submissions SET status = 'broadcast', txid = $1, updated_at = now()
                     WHERE id = $2",
                    &[txid, submission_id],
                )
                .await
                .map_err(|e| internal(e.into()))?;
        }
    }

    Ok(Json(ProcessBatchResp {
        broadcast: sent
            .into_iter()
            .map(|(submission_id, txid)| BroadcastEntry {
                submission_id,
                txid,
            })
            .collect(),
    }))
}

#[derive(serde::Deserialize)]
struct SplitNotesReq {
    /// Number of notes to split into (default 10).
    parts: Option<u32>,
    /// Zatoshis per note (default 0.002 ZEC, enough for ~20 record outputs each).
    zat_per_part: Option<u64>,
}

async fn split_notes(
    State(state): State<AppState>,
    Json(req): Json<SplitNotesReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let parts = req.parts.unwrap_or(10);
    let zat = req.zat_per_part.unwrap_or(200_000);
    let txid = state
        .wallet
        .split_notes(parts, zat)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "txid": txid, "parts": parts, "zat_per_part": zat })))
}

async fn rebuild(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let client = state.pg.get().await.map_err(|e| internal(e.into()))?;
    client
        .batch_execute("TRUNCATE address_book; TRUNCATE audit_records;")
        .await
        .map_err(|e| internal(e.into()))?;
    drop(client);

    let written = indexer::run_once(&state.wallet_db_path, &state.pg)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "rebuilt_records": written })))
}

async fn status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let s = state.wallet.status().await.map_err(internal)?;
    Ok(Json(serde_json::to_value(s).unwrap()))
}
