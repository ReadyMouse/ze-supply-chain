# crates/ — Rust Workspace

Three Rust crates form the backend: a shared memo codec, a key-holding wallet service, and a public API gateway.

## Layout

| Crate | Binary / lib | Purpose |
|---|---|---|
| `memo-schema` | library | Versioned 512-byte memo encode/decode (MessagePack + ZIP 302 marker) |
| `wallet-service` | binary | Org seed holder: ZIP 32 derivation, batch signing, block scan, indexer |
| `gateway` | binary | Public REST API: workers, records, admin; proxies wallet ops |

## Data flow

```
Gateway (no keys)  →  wallet-service (signer + indexer)  →  Zcash mainnet
       ↓                        ↓
   Postgres (ops)         Postgres (audit cache)
```

## Open-source candidacy

| Crate | Verdict |
|---|---|
| `memo-schema` | **Good candidate.** Only depends on `rmp`, `serde`, `thiserror`. No org secrets or proprietary APIs. |
| `gateway` | **Partial candidate.** Logic is generic, but tightly coupled to this project's Postgres schema and wallet-service API. |
| `wallet-service` | **Not a candidate.** Requires org BIP-39 seed, lightwalletd, and the full Zcash client stack. Key management is operator-specific. |

## Key files

- `memo-schema/src/lib.rs` — wire format, validation, annotated hex spans
- `wallet-service/src/wallet.rs` — spending actor, batch queue, lightwalletd sync
- `wallet-service/src/indexer.rs` — sqlite → Postgres audit export
- `gateway/src/main.rs` — public HTTP routes and `under_the_hood` memo preview

## Build & test

```bash
cargo build --workspace
cargo test --workspace
cargo run -p wallet-service   # port 7001
cargo run -p gateway          # port 7700
```
