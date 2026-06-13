# migrations/ — Database Schema

Postgres DDL applied idempotently at startup by both gateway and wallet-service.

## schema.sql

Creates four tables in two categories:

### Operational (gateway bookkeeping)

- **workers** — user_index, name, role, derived shielded address
- **submissions** — pending → broadcast → confirmed lifecycle with JSON payload

### Chain-derived (rebuildable from wallet sqlite + viewing key)

- **address_book** — enrollment memos decoded from chain
- **audit_records** — event memos decoded from chain (primary key: txid + pool + output_index)

## Rebuild semantics

`TRUNCATE address_book, audit_records` followed by wallet-service indexer pass fully reconstructs the audit ledger from on-chain memos. This is the "Postgres is not the source of truth" proof.

## Open-source candidacy

**Good candidate.** Generic Postgres schema with no proprietary extensions.
