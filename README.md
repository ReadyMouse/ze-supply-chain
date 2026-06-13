<!-- ZE Supply Chain — Main README
     Project overview, memo encoding spec, runbook, and component index.
     INPUT: None (primary project documentation)
     OUTPUT: Onboarding guide for developers and demo operators
     NOTES: See design.md, PLAN.md, and README_*.md in subfolders for detail.
     Written by Composer for Ze Supply Chain. June 2025. All rights reserved. -->

# ze-supply-chain

An immutable audit log for supply chains, powered by Zcash shielded transactions.
Every record — item arrivals, handoffs, inspections — is serialized into the
encrypted 512-byte memo of a mainnet shielded transaction. The blockchain is the
ledger; Postgres is a cache that can always be rebuilt from chain + viewing key.

See [design.md](design.md) for product context and [PLAN.md](PLAN.md) for the
implementation plan.

### Repository layout

| Folder | README | Contents |
|---|---|---|
| `crates/` | [README_crates.md](README_crates.md) | Rust workspace: memo-schema, wallet-service, gateway |
| `web/` | [README_web.md](README_web.md) | React demo UI (Vite + TypeScript) |
| `scripts/` | [README_scripts.md](README_scripts.md) | Demo seed script and mock wallet |
| `migrations/` | [README_migrations.md](README_migrations.md) | Postgres schema (operational + chain-derived tables) |

---

## How it works

### End-to-end flow

```
Worker UI  →  Gateway (no keys)  →  Wallet-service (signer + indexer)  →  Zcash mainnet
                ↓                         ↓
           Postgres (ops)            Postgres (audit)
           submissions,              address_book,
           user_index                  audit_records
```

1. **Enrollment** — A new worker gets a shielded address derived from the org
   seed via ZIP 32 (`m/32'/133'/N'` where `N` is the worker's `user_index`).
   An enrollment memo (name + role) is batched into a shielded transaction.
   The address *is* the identity — no user ID field goes in every event memo.

2. **Record submission** — A cold-chain event (item ID, type, quantity,
   temperature, notes) is validated by the gateway, encoded into a 512-byte
   memo, and queued. The wallet-service batches queued records into a single
   **send-many** transaction: one shielded output + one memo per record, all
   outputs targeting the submitting worker's derived address.

3. **Confirmation** — After ~75 seconds (mainnet block time), the transaction
   confirms. The indexer (same wallet-service process) scans compact blocks,
   trial-decrypts memos with the org viewing key, decodes the wire format, and
   writes to Postgres.

4. **Audit** — The dashboard reads the reconstructed ledger. Each row links to
   a real mainnet txid. **Rebuild from Chain** truncates the audit tables and
   repopulates them entirely from decrypted on-chain memos — the proof that
   Postgres is not the source of truth.

### Identity and what the chain provides for free

These deliberately stay **out** of the memo payload:

| Concern | Where it lives |
|---|---|
| Worker identity | The receiving z-address (ZIP 32 derivation + enrollment address book) |
| Org isolation | Which viewing key can decrypt the memo |
| Timestamp | Block time (authoritative); `client_ts` in the memo is advisory |
| Transaction ID, block height | Block metadata, recorded by the indexer |

Account index `0` (`m/32'/133'/0'`) is the org treasury; workers start at `1`.

### Batching

Records flush every `BATCH_MAX_AGE_SECS` (default 120s) or at
`BATCH_MAX_RECORDS` (default 5) — one shielded output + memo per record, many
records per transaction. ZIP 317 fees are per-output (~0.0001 ZEC per record).

---

## Memo binary encoding

Each shielded output carries a fixed **512-byte memo field**. Our payloads use
ZIP 302's binary-memo convention so third-party wallets do not try to render the
bytes as UTF-8 text.

Implementation lives in `crates/memo-schema` — the single source of truth shared
by the gateway (encode) and wallet-service (decode). The web UI's **Under the
Hood** panel shows the annotated hex dump of exactly what goes on-chain.

### Wire layout

```
byte 0:      0xFF            — ZIP 302 marker for "arbitrary binary data"
byte 1:      schema version  — u8, currently 1
bytes 2..:   MessagePack payload (positional arrays, no field names)
remainder:   zero padding to 512
```

The `0xFF` prefix follows ZIP 302: memos whose first byte is ≤ `0xF4` are
treated as UTF-8 text. The version byte sits *outside* MessagePack so a future
indexer can select the right decoder before parsing the body. MessagePack is
self-delimiting; trailing zero padding is ignored on decode.

### Payload format: positional arrays

Field names never go on the wire. Position defines meaning; `memo-schema` is the
contract for ordering.

**Enrollment** (type tag `0`):

```
[type, name, role]
```

Example: Alice Nguyen, warehouse worker → MessagePack array of 3 elements
starting with `0`, then two UTF-8 strings.

**Event** (type tag `1`):

```
[type, item_id, event_type, quantity, temp_centi, client_ts, notes]
```

| Field | Type | Notes |
|---|---|---|
| `type` | u8 | `0` = enroll, `1` = event |
| `item_id` | string | Max 64 bytes |
| `event_type` | u8 | `0` = received, `1` = handoff, `2` = inspection |
| `quantity` | u32 | Integer count |
| `temp_centi` | i32 | Temperature in centi-degrees Celsius (4.00°C → `400`) |
| `client_ts` | u32 | Unix seconds from the client; block time is authoritative |
| `notes` | string | Max 350 bytes |

`temp_centi` uses signed integer centi-degrees instead of floats — smaller on
the wire and no float-equality issues in the audit DB. Oversized fields are
**rejected** (not truncated) so workers see an error instead of silent data loss.

### Size budget

Worst-case event (64-byte item ID, 350-byte notes, numeric fields at max width)
uses ~440 bytes including the 2-byte header — ~70 bytes of headroom for future
fields. A unit test in `memo-schema` constructs this worst case and asserts it
fits with ≥50 bytes spare.

### Example (event record)

A received shipment for `LOT-2026-0042`, quantity 144, 4.00°C, with notes
`"received shipment, temp 4°C, seal intact"` uses 72 bytes on-chain (2-byte
header + 70-byte MessagePack body; remainder is zero padding). Annotated layout:

```
FF 01                          ← marker + version 1
97                             ← MessagePack fixarray[7]
01                             ← type tag: event
AD "LOT-2026-0042"             ← item_id (fixstr, 13 bytes)
00                             ← event_type: received
CC 90                          ← quantity: 144 (uint8)
CD 01 90                       ← temp_centi: 400 (uint16 on wire)
CE 6A 18 A5 00                 ← client_ts: 1780000000 (uint32)
D9 29 "received shipment…"     ← notes (str8, 41 bytes)
00 00 …                        ← zero padding to 512
```

The gateway's record-detail API returns `memo_hex` plus labelled `memo_spans`
for every byte range (marker, version, each field, padding). The web UI's
**Under the Hood** panel renders the same annotated dump live when you submit a
record — no need to read Rust to verify what goes on-chain.

---

## Components

| Path | What it is |
|---|---|
| `crates/memo-schema` | Versioned memo wire format (MessagePack, 512-byte budget) |
| `crates/wallet-service` | Signer + indexer: org seed, ZIP 32 derivation, batching, block scan, Postgres export |
| `crates/gateway` | Public API: workers, records, admin. Never sees keys |
| `web/` | React demo UI: New User / Log Temp / Audit + annotated memo viewer |
| `docker-compose.yml` | Postgres (Zcash node + lightwalletd operated separately) |

---

## Runbook

### Prerequisites

Rust (1.85+), Node 22+, Docker, and an operator-run lightwalletd gRPC endpoint
backed by a synced mainnet Zcash node.

### Infrastructure

```bash
docker compose up -d postgres
```

### Wallet seed

```bash
cargo run -p wallet-service -- gen-seed
cp .env.example .env
# paste the seed into WALLET_SEED_PHRASE, set WALLET_BIRTHDAY to the current
# mainnet block height (https://mainnet.zcashexplorer.app)
```

**The seed controls real ZEC. Never commit `.env`.**

Fund the org wallet: run the services once to log the org address (or hit
`GET /status`), then send it a small amount of ZEC — a few dollars covers
hundreds of records at ZIP 317 fee rates.

### Services

```bash
cargo run -p wallet-service   # signer + indexer, port 7001 (internal)
cargo run -p gateway          # public API, port 7700
cd web && npm install && npm run dev   # UI on http://localhost:5173
```

Set `LIGHTWALLETD_URL` in `.env` to your lightwalletd endpoint
(`http://localhost:9067` for the compose bridge). For development without a
local node, public endpoints (e.g. `https://zec.rocks:443`) speak the same
gRPC interface.

### Demo flow

1. **New User** — enrolls a worker; their ZIP 32-derived shielded address *is*
   their identity. The enrollment memo rides the next batch.
2. **Log Temp** — submit a cold-chain event. Queued (`pending`), batched into a
   send-many tx (`broadcast`), mined (`confirmed`, ~75s).
3. **Audit** — reconstructed ledger with per-record txids linking to a public
   block explorer. **Process Batch** broadcasts immediately instead of waiting
   for the batch timer.
4. **Rebuild from Chain** — truncates audit tables and reconstructs from
   decrypted on-chain memos.

### Note management

A note cannot be spent until its previous spend confirms. After funding, pre-split
ZEC so batches do not serialize behind a single change output:

```bash
curl -X POST localhost:7700/admin/split-notes \
  -H 'Content-Type: application/json' -d '{"parts": 10, "zat_per_part": 200000}'
```

### Demo seeding

With both services running and the wallet funded + split:

```bash
./scripts/seed-demo.sh   # enrolls 3 workers, submits 4 events, broadcasts both batches
```

### Tests

```bash
cargo test --workspace        # includes the 512-byte memo budget tests
```
