//! Public API gateway. Validates submissions, owns the workers/submissions
//! tables, and proxies wallet operations to the wallet-service. Never sees keys.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::{Manager, Pool};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio_postgres::NoTls;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use memo_schema::{EventRecord, EventType, Record};

/// Judge-facing construction details: the real bytes that will go on-chain.
fn under_the_hood(
    record: &Record,
    user_index: i32,
    address: &str,
    receiver_name: &str,
    receiver_role: &str,
) -> Result<serde_json::Value, ApiError> {
    let (memo, spans) =
        memo_schema::encode_memo_annotated(record).map_err(|e| internal(e.to_string()))?;
    // The shape handed to the wallet-service, which becomes a ZIP 321 payment
    // in the proposed transaction sent to lightwalletd.
    let payment_json = serde_json::json!({
        "spend_from_account": "m/32'/133'/0'",
        "payments": [{
            "recipient_address": address,
            "value_zat": 10_000,
            "memo_plaintext": record,
            "memo_encoded": "512-byte MessagePack buffer (annotated below)",
        }],
        "fee_rule": "ZIP-317",
    });
    Ok(serde_json::json!({
        "derivation_path": format!("m/32'/133'/{user_index}'"),
        "address": address,
        "sender": {
            "label": "org treasury",
            "derivation_path": "m/32'/133'/0'",
        },
        "receiver": {
            "label": receiver_name,
            "role": receiver_role,
            "derivation_path": format!("m/32'/133'/{user_index}'"),
            "address": address,
        },
        "payment_json": payment_json,
        "memo_hex": hex::encode(memo),
        "memo_spans": spans,
        "record_json": record,
    }))
}

#[derive(Clone)]
struct AppState {
    pg: Pool,
    http: reqwest::Client,
    wallet_url: Arc<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,gateway=debug".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL")?;
    let wallet_addr =
        std::env::var("WALLET_SERVICE_ADDR").unwrap_or_else(|_| "127.0.0.1:7001".into());
    // 7700: avoids macOS ControlCenter, which squats on 5000/7000 for AirPlay.
    let listen_addr = std::env::var("GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:7700".into());

    let config: tokio_postgres::Config = database_url.parse().context("parse DATABASE_URL")?;
    let pg = Pool::builder(Manager::new(config, NoTls))
        .max_size(8)
        .build()
        .context("pg pool")?;
    // Apply the schema idempotently so the gateway can boot without the
    // wallet-service (which also applies it) being up yet.
    pg.get()
        .await
        .context("connect postgres")?
        .batch_execute(include_str!("../../../migrations/schema.sql"))
        .await
        .context("apply schema")?;

    let state = AppState {
        pg,
        http: reqwest::Client::new(),
        wallet_url: Arc::new(format!("http://{wallet_addr}")),
    };

    let app = Router::new()
        .route("/workers", get(list_workers).post(create_worker))
        .route("/records", get(list_records).post(create_record))
        .route("/records/{id}", get(get_record))
        .route("/admin/process-batch", post(process_batch))
        .route("/admin/rebuild", post(rebuild))
        .route("/admin/split-notes", post(split_notes))
        .route("/status", get(status))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .with_context(|| format!("bind {listen_addr}"))?;
    tracing::info!("gateway listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

type ApiError = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, msg: impl std::fmt::Display) -> ApiError {
    (status, Json(serde_json::json!({ "error": msg.to_string() })))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!("internal error: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, e)
}

fn ts(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

// ---------- workers ----------

#[derive(serde::Serialize)]
struct Worker {
    user_index: i32,
    name: String,
    role: String,
    address: String,
    enrolled: bool,
    enroll_status: Option<String>,
}

async fn list_workers(State(state): State<AppState>) -> Result<Json<Vec<Worker>>, ApiError> {
    let client = state.pg.get().await.map_err(internal)?;
    let rows = client
        .query(
            "SELECT w.user_index, w.name, w.role, w.address,
                    EXISTS (SELECT 1 FROM address_book ab WHERE ab.address = w.address) AS enrolled,
                    s.status
             FROM workers w
             LEFT JOIN submissions s
               ON s.user_index = w.user_index AND s.kind = 'enroll'
             ORDER BY w.user_index",
            &[],
        )
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.iter()
            .map(|r| Worker {
                user_index: r.get(0),
                name: r.get(1),
                role: r.get(2),
                address: r.get(3),
                enrolled: r.get(4),
                enroll_status: r.get(5),
            })
            .collect(),
    ))
}

#[derive(serde::Deserialize)]
struct CreateWorker {
    name: String,
    role: String,
}

async fn create_worker(
    State(state): State<AppState>,
    Json(req): Json<CreateWorker>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "name is required"));
    }

    let client = state.pg.get().await.map_err(internal)?;
    // user_index 0 is the org treasury; workers start at 1.
    let row = client
        .query_one(
            "SELECT COALESCE(MAX(user_index), 0) + 1 FROM workers",
            &[],
        )
        .await
        .map_err(internal)?;
    let user_index: i32 = row.get(0);
    let submission_id = Uuid::new_v4();

    // Ask the wallet service to derive the address and queue the enrollment memo.
    let resp = state
        .http
        .post(format!("{}/enroll", state.wallet_url))
        .json(&serde_json::json!({
            "user_index": user_index as u32,
            "name": req.name,
            "role": req.role,
            "submission_id": submission_id,
        }))
        .send()
        .await
        .map_err(internal)?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(internal(format!("wallet-service enroll failed: {body}")));
    }
    let enroll: serde_json::Value = resp.json().await.map_err(internal)?;
    let address = enroll["address"].as_str().unwrap_or_default().to_string();

    client
        .execute(
            "INSERT INTO workers (user_index, name, role, address) VALUES ($1, $2, $3, $4)",
            &[&user_index, &req.name, &req.role, &address],
        )
        .await
        .map_err(internal)?;
    client
        .execute(
            "INSERT INTO submissions (id, user_index, kind, payload, status)
             VALUES ($1, $2, 'enroll', $3, 'pending')",
            &[
                &submission_id,
                &user_index,
                &serde_json::json!({ "name": req.name, "role": req.role }),
            ],
        )
        .await
        .map_err(internal)?;

    let enroll_record = Record::Enroll(memo_schema::EnrollRecord {
        name: req.name.clone(),
        role: req.role.clone(),
    });
    let hood = under_the_hood(&enroll_record, user_index, &address, &req.name, &req.role)?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "user_index": user_index,
            "address": address,
            "submission_id": submission_id,
            "status": "pending",
            "under_the_hood": hood,
        })),
    ))
}

// ---------- records ----------

#[derive(serde::Deserialize)]
struct CreateRecord {
    user_index: u32,
    item_id: String,
    event_type: EventType,
    quantity: u32,
    /// Temperature in °C; converted to centi-degrees on the wire.
    temp_c: f64,
    #[serde(default)]
    notes: String,
}

async fn create_record(
    State(state): State<AppState>,
    Json(req): Json<CreateRecord>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let record = Record::Event(EventRecord {
        item_id: req.item_id,
        event_type: req.event_type,
        quantity: req.quantity,
        temp_centi: (req.temp_c * 100.0).round() as i32,
        client_ts: OffsetDateTime::now_utc().unix_timestamp() as u32,
        notes: req.notes,
    });
    // Reject anything that won't fit in a memo before it enters the pipeline.
    memo_schema::encode_memo(&record)
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let client = state.pg.get().await.map_err(internal)?;
    let worker = client
        .query_opt(
            "SELECT address, name, role FROM workers WHERE user_index = $1",
            &[&(req.user_index as i32)],
        )
        .await
        .map_err(internal)?;
    let Some(worker) = worker else {
        return Err(err(StatusCode::NOT_FOUND, "unknown worker"));
    };
    let address: String = worker.get(0);
    let worker_name: String = worker.get(1);
    let worker_role: String = worker.get(2);

    let submission_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO submissions (id, user_index, kind, payload, status)
             VALUES ($1, $2, 'event', $3, 'pending')",
            &[
                &submission_id,
                &(req.user_index as i32),
                &serde_json::to_value(&record).unwrap(),
            ],
        )
        .await
        .map_err(internal)?;

    let resp = state
        .http
        .post(format!("{}/submit", state.wallet_url))
        .json(&serde_json::json!({
            "submission_id": submission_id,
            "user_index": req.user_index,
            "record": record,
        }))
        .send()
        .await
        .map_err(internal)?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(internal(format!("wallet-service submit failed: {body}")));
    }

    let hood = under_the_hood(
        &record,
        req.user_index as i32,
        &address,
        &worker_name,
        &worker_role,
    )?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": submission_id,
            "status": "pending",
            "under_the_hood": hood,
        })),
    ))
}

#[derive(serde::Deserialize)]
struct RecordFilters {
    user_index: Option<i32>,
    event_type: Option<String>,
    item_id: Option<String>,
}

#[derive(serde::Serialize)]
struct AuditRecord {
    txid: String,
    block_height: i64,
    block_time: Option<String>,
    worker_name: Option<String>,
    worker_role: Option<String>,
    user_index: Option<i32>,
    item_id: String,
    event_type: String,
    quantity: i64,
    temp_c: f64,
    notes: String,
}

/// Confirmed, chain-derived records plus any submissions still in flight.
async fn list_records(
    State(state): State<AppState>,
    Query(f): Query<RecordFilters>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = state.pg.get().await.map_err(internal)?;

    let rows = client
        .query(
            "SELECT ar.txid, ar.block_height, ar.block_time, ab.name, ab.role,
                    ar.user_index, ar.item_id, ar.event_type, ar.quantity,
                    ar.temp_centi, ar.notes
             FROM audit_records ar
             LEFT JOIN address_book ab ON ab.address = ar.address
             WHERE ($1::int IS NULL OR ar.user_index = $1)
               AND ($2::text IS NULL OR ar.event_type = $2)
               AND ($3::text IS NULL OR ar.item_id = $3)
             ORDER BY ar.block_height DESC, ar.txid",
            &[&f.user_index, &f.event_type, &f.item_id],
        )
        .await
        .map_err(internal)?;

    let confirmed: Vec<AuditRecord> = rows
        .iter()
        .map(|r| AuditRecord {
            txid: r.get(0),
            block_height: r.get(1),
            block_time: r.get::<_, Option<OffsetDateTime>>(2).map(ts),
            worker_name: r.get(3),
            worker_role: r.get(4),
            user_index: r.get(5),
            item_id: r.get(6),
            event_type: r.get(7),
            quantity: r.get(8),
            temp_c: r.get::<_, i32>(9) as f64 / 100.0,
            notes: r.get(10),
        })
        .collect();

    // In-flight submissions (pending or broadcast, not yet seen on-chain).
    let pending_rows = client
        .query(
            "SELECT s.id, s.user_index, s.kind, s.payload, s.status, s.txid, s.created_at, w.name
             FROM submissions s
             LEFT JOIN workers w ON w.user_index = s.user_index
             WHERE s.status <> 'confirmed'
             ORDER BY s.created_at DESC",
            &[],
        )
        .await
        .map_err(internal)?;
    let pending: Vec<serde_json::Value> = pending_rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<_, Uuid>(0),
                "user_index": r.get::<_, i32>(1),
                "kind": r.get::<_, String>(2),
                "payload": r.get::<_, serde_json::Value>(3),
                "status": r.get::<_, String>(4),
                "txid": r.get::<_, Option<String>>(5),
                "created_at": ts(r.get(6)),
                "worker_name": r.get::<_, Option<String>>(7),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "confirmed": confirmed,
        "in_flight": pending,
    })))
}

async fn get_record(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = state.pg.get().await.map_err(internal)?;
    let row = client
        .query_opt(
            "SELECT id, user_index, kind, payload, status, txid, created_at, updated_at
             FROM submissions WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(internal)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "no such submission"))?;

    Ok(Json(serde_json::json!({
        "id": row.get::<_, Uuid>(0),
        "user_index": row.get::<_, i32>(1),
        "kind": row.get::<_, String>(2),
        "payload": row.get::<_, serde_json::Value>(3),
        "status": row.get::<_, String>(4),
        "txid": row.get::<_, Option<String>>(5),
        "created_at": ts(row.get(6)),
        "updated_at": ts(row.get(7)),
    })))
}

// ---------- admin ----------

async fn proxy_post(
    state: &AppState,
    path: &str,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = state
        .http
        .post(format!("{}{path}", state.wallet_url))
        .send()
        .await
        .map_err(internal)?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    if !status.is_success() {
        return Err(internal(format!("wallet-service {path} failed: {body}")));
    }
    Ok(Json(body))
}

async fn process_batch(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    proxy_post(&state, "/process-batch").await
}

async fn rebuild(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    proxy_post(&state, "/rebuild").await
}

async fn split_notes(
    State(state): State<AppState>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let payload = body.map(|Json(v)| v).unwrap_or(serde_json::json!({}));
    let resp = state
        .http
        .post(format!("{}/split-notes", state.wallet_url))
        .json(&payload)
        .send()
        .await
        .map_err(internal)?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    if !status.is_success() {
        return Err(internal(format!("wallet-service /split-notes failed: {body}")));
    }
    Ok(Json(body))
}

async fn status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = state
        .http
        .get(format!("{}/status", state.wallet_url))
        .send()
        .await
        .map_err(internal)?;
    let body: serde_json::Value = resp.json().await.map_err(internal)?;
    Ok(Json(body))
}
