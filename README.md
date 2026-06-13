# ze-supply-chain

An immutable audit logging system for supply chains, powered by Zcash's shielded
transaction layer. Every record — item arrivals, handoffs, inspections — is
serialized into the encrypted 512-byte memo of a shielded mainnet transaction.
The blockchain is the ledger; Postgres is just a cache that can always be
rebuilt from chain + viewing key.

See [design.md](design.md) for the architecture and [PLAN.md](PLAN.md) for the
implementation plan.

## Components

| Path | What it is |
|---|---|
| `crates/memo-schema` | Versioned memo wire format (MessagePack, 512-byte budget), shared by gateway + wallet-service |
| `crates/wallet-service` | Signer + indexer: holds the org seed, derives per-user ZIP 32 accounts, batches records into send-many transactions, scans blocks, exports decrypted memos to Postgres |
| `crates/gateway` | Public API: workers, records, admin (process-batch / rebuild / status). Never sees keys |
| `web/` | React demo UI: three-button home (New User / Log Temp / Audit) + audit dashboard |
| `docker-compose.yml` | Postgres (node + lightwalletd are operated separately) |

## Runbook

### 0. Prerequisites

Rust (1.85+), Node 22+, Docker, and an operator-run lightwalletd gRPC endpoint
backed by a synced mainnet Zcash node.

### 1. Infrastructure

```bash
docker compose up -d postgres
```

### 2. Wallet seed

```bash
cargo run -p wallet-service -- gen-seed
cp .env.example .env
# paste the seed into WALLET_SEED_PHRASE, set WALLET_BIRTHDAY to the current
# mainnet block height (https://mainnet.zcashexplorer.app)
```

**The seed controls real ZEC. Never commit `.env`.**

Fund the org wallet: run the services once to log the org address (or hit
`GET /status`), then send it a small amount of ZEC — a few dollars covers
hundreds of records at ZIP 317 fee rates (~0.0001 ZEC per record output).

### 3. Services

```bash
cargo run -p wallet-service   # signer + indexer, port 7001 (internal)
cargo run -p gateway          # public API, port 7700
cd web && npm install && npm run dev   # UI on http://localhost:5173
```

Set `LIGHTWALLETD_URL` in `.env` to your lightwalletd endpoint
(`http://localhost:9067` for the compose bridge). If you ever need to develop
without the node, any public endpoint (e.g. `https://zec.rocks:443`) speaks
the same gRPC interface.

### 4. Demo flow

1. **New User** — enrolls a worker; their ZIP 32-derived shielded address *is*
   their identity. The enrollment memo rides the next batch.
2. **Log Temp** — submit a cold-chain event. It's queued (`pending`), batched
   into a send-many tx (`broadcast`), and mined (`confirmed`, ~75s).
3. **Audit** — the reconstructed ledger with per-record txids linking to a
   public block explorer. Use **⚡ Process Batch** to broadcast immediately
   instead of waiting for the batch timer.
4. **♻ Rebuild from Chain** — truncates the audit tables and reconstructs them
   entirely from decrypted on-chain memos. The proof of the whole concept.

### Batching policy

Records flush every `BATCH_MAX_AGE_SECS` (default 120s) or at
`BATCH_MAX_RECORDS` (default 5), whichever comes first — one shielded output +
memo per record, many records per transaction.

### Note management

A note can't be spent until its previous spend confirms. After funding the
wallet, pre-split the ZEC so batches don't serialize behind a single change
output:

```bash
curl -X POST localhost:7700/admin/split-notes \
  -H 'Content-Type: application/json' -d '{"parts": 10, "zat_per_part": 200000}'
```

### Demo seeding

With both services running and the wallet funded + split:

```bash
./scripts/seed-demo.sh   # enrolls 3 workers, submits 4 events, broadcasts both batches
```

## Tests

```bash
cargo test --workspace        # includes the 512-byte memo budget tests
```
