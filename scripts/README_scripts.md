# scripts/ — Operator Utilities

Shell and Python helpers for demo setup and offline development.

## Files

| Script | Purpose |
|---|---|
| `seed-demo.sh` | Enrolls 3 workers, submits 4 events, splits notes, broadcasts batches via curl |
| `mock-wallet.py` | Fake wallet-service HTTP server for UI dev without a Zcash node |

## Usage

```bash
# Full demo seed (gateway + wallet-service must be running, wallet funded)
./scripts/seed-demo.sh
./scripts/seed-demo.sh http://localhost:7700

# UI-only dev (point gateway WALLET_SERVICE_ADDR at mock)
python3 scripts/mock-wallet.py 7001
```

## Open-source candidacy

**Good candidate.** Self-contained utilities with no proprietary secrets. `mock-wallet.py` is explicitly labelled fake data (`u1mock` addresses).
