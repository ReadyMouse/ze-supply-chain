<!-- Implementation Plan
     Hackathon-scoped build plan: Rust workspace layout, memo encoding, services.
     INPUT: design.md concept
     OUTPUT: Phased tasks, tech choices, workspace file map
     NOTES: Some items (sqlx) superseded by tokio-postgres in actual implementation.
     Written by Composer for Ze Supply Chain. June 2025. All rights reserved. -->

# Implementation Plan: Zcash Audit Log (Rust + TypeScript)

Implementation plan for the system described in [design.md](design.md), scoped to the 2–3 week hackathon vertical slice (pharma cold chain demo).

## Tech foundation

**Zcash stack.** Build the wallet logic on the official `librustzcash` crates rather than shelling out to a node wallet (`zcashd` is fully deprecated):

- `zcash_client_backend` — transaction proposal/creation, compact-block scanning, trial decryption
- `zcash_client_sqlite` — ready-made wallet store (notes, witnesses, scan checkpoints); use it instead of writing our own note management
- `zcash_primitives` / `zip32` — ZIP 32 HD derivation for per-user addresses and the UFVK
- `tonic` + `prost` — gRPC client for `lightwalletd` (or Zaino), the bridge to the network

Run against self-hosted `lightwalletd` endpoint. Light-client sync from a recent "birthday" height takes seconds — create the org wallet with a birthday of the current block height and never sync history.

**Services stack.** `tokio` + `axum` for HTTP, `sqlx` for Postgres, `rmp-serde` for MessagePack memos. No real auth — demo identity is an "act as worker" dropdown. Frontend: React + Vite + TypeScript, one app with two pages (three-button home, audit dashboard).

**Key simplification:** merge the signing service and indexer into a single `wallet-service` binary for the hackathon. Both need the same wallet database (the spending key holder must scan blocks anyway to track its own spendable notes), so separating them buys security-boundary points the demo doesn't need and costs a week of plumbing. The API gateway stays separate so the "spending key never touches the web tier" story remains true.

## Workspace layout

```
ze-supply-chain/
├── Cargo.toml                 # workspace
├── crates/
│   ├── memo-schema/           # shared payload types + MessagePack codec
│   ├── gateway/               # axum API: auth, validation, record queue
│   └── wallet-service/        # signer + indexer: ZIP 32 derivation, batching, send-many, block scan, Postgres writes
├── web/                       # React + Vite (worker form + dashboard)
├── migrations/                # sqlx migrations for the audit DB
└── docker-compose.yml         # Postgres (node + lightwalletd operated separately)
```

## Phase 0 — De-risking spike (days 1–3)

This is the make-or-break phase; everything else is ordinary web dev.

1. Stand up Postgres via docker-compose; point services at the self-hosted `lightwalletd` endpoint; fund the org wallet with a small amount of ZEC (a few dollars' worth covers hundreds of transactions at ZIP 317 fee rates).
2. Write a throwaway Rust binary that: creates a wallet from a seed with `zcash_client_sqlite`, syncs compact blocks, sends a shielded tx to itself with a MessagePack memo, then scans and decrypts that memo with the UFVK.
3. Verify ZIP 32 sub-account derivation: derive addresses for `user_index = 0, 1, 2`, send to each, confirm the scanner attributes notes to the right address.

**Exit criteria:** round-trip of an encrypted memo through mainnet, end to end. If any of this fights us, we find out on day 3, not day 12.

## Phase 1 — `memo-schema` crate (days 3–4)

- Versioned envelope: `{ version: u8, type: RecordType, body }` with `RecordType::Enroll | Event`.
- Cold-chain event body: item ID, event type (received/handoff/inspection), quantity, temperature, notes, client timestamp.
- MessagePack via `rmp-serde`, with a hard test that representative payloads stay under 512 bytes (truncate/reject notes that overflow).
- This crate is the contract between gateway and wallet-service — both depend on it, so schema drift is impossible.

### Memo wire format

The memo field is a fixed 512 bytes, zero-padded, with ZIP 302 conventions for the first byte.

```
byte 0:      0xFF            — ZIP 302 marker for "arbitrary binary data"
byte 1:      schema version  — u8, starts at 1
bytes 2..:   MessagePack payload (≤ 510 bytes)
remainder:   zero padding to 512 (MessagePack is self-delimiting; padding is ignored on decode)
```

The `0xFF` prefix keeps third-party wallets from rendering our binary as text (ZIP 302 treats first byte ≤ `0xF4` as UTF-8). The version byte lives *outside* the MessagePack body so a future indexer can pick the right decoder before parsing anything.

**Payload encoding: positional arrays, not maps.** Field names never go on the wire — position defines meaning, and `memo-schema` is the single source of truth for ordering:

```
Event  (type tag 1): [type, item_id, event_type, quantity, temp_centi, client_ts, notes]
Enroll (type tag 0): [type, name, role]
```

Field-level choices:

- `event_type` — u8 enum (0 = received, 1 = handoff, 2 = inspection), not a string.
- `temp_centi` — temperature as a signed int in centi-degrees (4.0°C → 400); avoids float encoding overhead and float-equality weirdness in the audit DB.
- `client_ts` — unix seconds as u32; the authoritative timestamp is the block time, this just records what the device thought.
- `item_id` — string, capped at 64 bytes.
- `notes` — capped at 350 bytes, enforced in the gateway *and* in `memo-schema` (reject, don't truncate, so the worker sees the error instead of silently losing data).

**What deliberately stays out of the memo** — the chain provides it for free:

- Worker identity: the receiving z-address *is* the identity (ZIP 32 derivation + enrollment address book).
- Org ID: implied by which viewing key can decrypt the memo at all.
- Timestamp / txid / block height: block metadata, recorded by the indexer, unforgeable.

**Size budget:** worst-case event (64-byte item ID, 350-byte notes, all numeric fields at max width) ≈ 440 bytes including header — ~70 bytes of headroom for future fields. A unit test constructs this worst case and asserts it fits. Oversized records are rejected at the gateway; multi-output continuation stays out of hackathon scope.

## Phase 2 — `wallet-service`: signing side (days 4–8)

- Load seed from env/secrets file; hold the spending key only in this process.
- **Enrollment endpoint** (internal HTTP): given `user_index` + name + role, derive the z-address, broadcast the enrollment memo tx, return the address.
- **Record queue consumer:** gateway POSTs serialized records to an internal endpoint; wallet-service buffers them and flushes on a timer (e.g. every 2 min) or count threshold (e.g. 5 records), building one send-many transaction with one output + memo per record via `propose_transfer` → `create_proposed_transactions`.
- **Note management gotcha:** a freshly funded wallet has one note, and a note can't be spent until the tx spending it confirms (~75s). Pre-split the funding ZEC into ~20 small notes during setup so batches don't serialize behind each other.
- ZIP 317 fees are per-output and now cost real money — still tiny (≈0.0001 ZEC per logical action), but log fees per transaction so spend is visible and the billing story is demo-able.
## Phase 3 — `wallet-service`: indexer side (days 8–11)

- Continuous scan loop: `scan_cached_blocks` against lightwalletd, checkpointing height (the sqlite wallet store gives resumability for free — covers the "basic checkpointing" scope).
- After each scan, pull newly decrypted memos, deserialize with `memo-schema`, and write to Postgres:
  - `enroll` memos → upsert into `address_book` (address, name, role, active)
  - `event` memos → insert into `audit_records` (all fields + txid, block height, block time, derived address → joined worker identity)
- Records the service itself sent are decryptable since outputs go to addresses under the org's own UFVK — one viewing key covers everything.
- Skip reorg handling beyond what the wallet store does natively (per design.md's out-of-scope list).

## Phase 4 — `gateway` (days 10–13, overlaps Phase 3)

No real auth — this is a tech demo, so identity is an "act as worker" selection rather than login/JWT/password hashing.

- `GET /workers` — list enrolled workers (from the address book) for the act-as dropdown, with enrollment status.
- `POST /workers` — name + role; creates the DB row and triggers wallet-service enrollment (itself an on-chain tx, so the response carries a pending status).
- `POST /records` — validate payload, serialize via `memo-schema`, forward to wallet-service with the selected `user_index`. Return a record UUID immediately (status: pending).
- `GET /records` — query the audit DB with filters (worker, event type, item, date range); each row includes txid and status.
- `POST /admin/process-batch` — force the wallet-service batcher to broadcast the current batch immediately (demo pacing).
- `POST /admin/rebuild` — truncate audit tables and trigger a rescan from the wallet birthday.
- `GET /status` — block height, wallet balance, spendable note count, records waiting in the current batch.

## Phase 5 — Frontend (days 12–17)

One Vite app, two pages. Slim tech demo, not a product UI.

**Home page — exactly three buttons:**

- **[New User]** — modal/form: name + role → enrolls via on-chain tx. Shows a pending indicator until the enrollment confirms (~75s), since enrollment is itself a transaction.
- **[Log Temp]** — form: act-as worker dropdown, item ID, temp, notes → submit. On submit, show the record's lifecycle indicator (pending → batched → broadcast → confirmed) — this is a status chip, not a button, and it's how the architecture stays visible in the demo.
- **[Audit]** — navigates to the dashboard.

**Audit dashboard — the technical display lives here:**

- Filterable table of records: worker (from address book), event details, block time, status chip per row, and the txid linked to a mainnet block explorer — that link is the demo's money shot ("click here, the record is on a public blockchain, encrypted").
- System status strip: current block height, wallet balance, spendable notes, records waiting in the current batch.
- **[Process Batch]** — forces the current batch to broadcast immediately, for demo pacing.
- **[Rebuild from Chain]** — truncates the audit tables and rescans from the wallet birthday, repopulating the dashboard from nothing but chain + viewing key, on camera.

## Phase 6 — Demo recording & hardening (days 17–21)

- Seed script: create org wallet, pre-split notes, enroll 2–3 demo workers, submit a handful of historical events so the dashboard isn't empty.
- **Recorded demo** (no live presentation): script and record the full flow — worker submits an event, batch broadcasts, ~75s confirmation, record appears in the dashboard with its txid. Recording means confirmation waits can be cut or time-lapsed, but every txid shown must be a real, confirmed mainnet transaction that judges can look up themselves afterward.
- **Verification handout for judges:** a short doc listing the demo txids with explorer links so they can independently confirm the batch transactions exist on-chain. Consider including the org viewing key with decryption instructions for the truly motivated.
- **"Rebuild from chain" segment:** wipe the audit tables on camera, rerun the indexer from the wallet birthday, watch Postgres repopulate — proves the core claim of the product and costs almost nothing (`TRUNCATE` + rescan).
- README with runbook; docker-compose for everything but the frontend dev server.

## Main risks, in order

1. **Node availability** — the operator's node must be synced and reachable (via lightwalletd/Zaino) before mainnet operations; development can bridge through a public gRPC endpoint in the meantime.
2. **librustzcash API churn** — the crates are actively evolving; pin versions on day 1 and budget Phase 0 generously.
3. **Real funds on mainnet** — the seed now controls actual ZEC. Keep the float small (a few dollars), back up the seed phrase securely, and never commit it; everything is rebuildable from seed + chain.
4. **Confirmation latency** — ~75s block times are mostly defused by the recorded demo (cut or time-lapse the wait), but the pending → confirmed status UI is still worth building since it's how the product would actually feel.
5. **Note fragmentation/concurrency** — handled by pre-splitting, but if batches ever fail with "insufficient funds" despite a balance, this is why.
6. **Memo overflow** — enforced in `memo-schema` with tests, but keep the event form's notes field capped in the UI too.
