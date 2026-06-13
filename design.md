<!-- Product Design Overview
     High-level architecture for the Zcash audit log supply-chain system.
     INPUT: None (design document)
     OUTPUT: Component descriptions and data flow for implementers
     NOTES: Companion to PLAN.md (implementation) and README.md (runbook).
     Written by Composer for Ze Supply Chain. June 2025. All rights reserved. -->

# Zcash Audit Log System — Component Overview

## Concept Summary

An immutable audit logging system for supply chains and chain-of-custody use cases, powered by Zcash's shielded transaction layer. Every record update — item arrivals, handoffs, inspections — gets serialized into the encrypted memo field of a shielded Zcash transaction and broadcast on-chain. The blockchain becomes an unforgeable, timestamped ledger that only authorized parties can read. An indexer scans blocks and reconstructs a queryable local database from those encrypted memos, so clients get both cryptographic integrity and a normal database interface. All key management and devops is handled by the operator; clients pay a SaaS fee and never touch a wallet.

**Target customers:** pharma cold chain, legal evidence intake, franchise inspection logs — anywhere the question "prove this record wasn't tampered with" has real stakes.

---

## Components

### 1. Worker App (Frontend)
- Web-based login + authentication (worker identity, org routing)
- Event entry form (item ID, event type, quantity, notes, timestamp)
- Submits records to the API gateway

### 2. API Gateway
- Validates and authenticates incoming records
- Serializes payloads into compact format (MessagePack or tight JSON) to fit within the 512-byte memo limit
- Batches records and forwards to the signing service
- Enforces worker identity so records can't be spoofed at the application layer

### 3. Signing Service
- Holds the org's Zcash master spending key (never exposed to the web tier)
- Derives unique z-addresses per user via ZIP 32 hierarchical deterministic derivation (`m/purpose'/coin'/org_id'/user_index'`) — no user ID needed in the memo field
- Constructs and broadcasts send-many transactions
- Each output targets the user's derived z-address; memo field carries the record payload
- Fires on a schedule (every N minutes) or threshold (every N records) to batch multiple records into a single transaction and minimize fees

### 4. Zcash Network
- Provides immutable, timestamped, encrypted on-chain storage
- Shielded transactions keep record contents private to viewing key holders
- Block confirmation (~75 seconds) is the write finality window

### 5. Indexer Service
- Scans new blocks continuously
- Decrypts memos using the org's full viewing key
- Parses and validates record payloads
- Writes reconstructed records to Postgres
- Handles reorgs and checkpoints block height for resumability

### 6. Audit Database (Postgres)
- Queryable local reconstruction of the on-chain record set
- Source of truth for the dashboard and any client integrations
- Can always be wiped and fully rebuilt from chain using the viewing key

### 7. Audit Dashboard (Frontend)
- Displays reconstructed ledger per org
- Filterable by worker, event type, item, and date range
- Exposes the on-chain transaction ID per record for independent verification

### 8. Fee & Billing Layer
- SaaS subscription per org
- Covers transaction fees (ZEC float per org) plus margin
- Auto-tops-up ZEC balance as needed

---

## Key Cross-Cutting Concerns

### Key Management
- Spending key lives in a secrets manager, never exposed to the web tier
- Full viewing key distributed read-only to auditors and compliance stakeholders
- Viewing key and spending key are fully separable — read and write access are independent

### Memo Schema
- Versioned, compact payload format (MessagePack recommended)
- Schema shared and versioned across the API gateway and indexer
- Each memo must fit within 512 bytes; send-many outputs can be used for larger records

### User Identity & Address Derivation
- Each user gets a unique z-address derived from the org's master key via ZIP 32 — the address *is* the identity, no user ID field needed in every memo
- Derivation path: `m/purpose'/coin'/org_id'/user_index'` — re-derived on demand, no per-user key storage
- On enrollment, the system derives the user's address and broadcasts a one-time enrollment transaction; memo payload is `{"type":"enroll","name":"Alice Nguyen","role":"warehouse_worker"}`
- The indexer builds an address book table from enrollment memos; all subsequent records from that address are attributed automatically
- Enrollment transactions are flagged as a special record type and treated with extra care by the indexer (rebroadcast on re-index if missing)
- If a user leaves or an address is compromised, derive a new address, issue a new enrollment transaction, and mark the old address inactive in the address book — historical records remain intact and readable
- ZIP 32 viewing keys can be scoped to a subtree, so an org admin sees all their users' transactions without visibility into other orgs

### Multi-Tenancy
- Org isolation at the z-address level (each org gets its own address set)
- Database-level isolation in Postgres (schema or row-level by org ID)

---

## Hackathon Scope (2-3 weeks)

A single vertical slice is the target — pharma cold chain is the recommended demo scenario.

**MVP flow:**
1. Worker logs in and submits a cold chain event (e.g. "received shipment, temp 4°C")
2. API serializes and queues the record
3. Signing service batches and broadcasts a send-many transaction
4. Indexer picks up the confirmed block, decrypts the memo, writes to Postgres
5. Audit dashboard shows the reconstructed ledger entry with its on-chain tx ID

**Out of scope for hackathon:** billing automation, multi-org provisioning, reorg handling beyond basic checkpointing, mobile worker app.