#!/usr/bin/env bash
# Demo Seed Script
#
#   Enrolls three demo workers, submits four cold-chain events, and broadcasts
#   both batches via the gateway admin API. Requires funded wallet + services.
#
# INPUT:
#   - Optional gateway URL (default http://localhost:7700)
#   - Running gateway (7700) and wallet-service (7001)
#
# OUTPUT:
#   - curl POSTs to /workers, /records, /admin/split-notes, /admin/process-batch
#
# NOTES:
#   Records confirm ~75s after broadcast. Splits treasury into 10 notes first.
#
# Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

# Seed the demo: enroll workers and submit a believable cold-chain history.
# Requires gateway (7700) + wallet-service (7001) running and a funded wallet.
#
# Usage: ./scripts/seed-demo.sh [gateway-url]
set -euo pipefail

GW="${1:-http://localhost:7700}"

say() { printf '\n\033[1;33m== %s\033[0m\n' "$*"; }

post() { # post <path> <json>
  curl -sS -X POST "$GW$1" -H 'Content-Type: application/json' -d "$2"
  echo
}

say "Wallet status"
curl -sS "$GW/status"; echo

say "Splitting treasury into 10 notes (batches won't serialize behind change)"
post /admin/split-notes '{"parts": 10, "zat_per_part": 200000}'

say "Enrolling demo workers"
post /workers '{"name": "Alice Nguyen", "role": "warehouse_worker"}'
post /workers '{"name": "Bob Okafor", "role": "driver"}'
post /workers '{"name": "Carmen Diaz", "role": "inspector"}'

say "Broadcasting enrollment batch"
post /admin/process-batch '{}'

say "Submitting cold-chain events (worker indices 1-3)"
post /records '{"user_index": 1, "item_id": "LOT-2026-0042", "event_type": "received", "quantity": 144, "temp_c": 4.0, "notes": "received shipment, seal intact"}'
post /records '{"user_index": 2, "item_id": "LOT-2026-0042", "event_type": "handoff", "quantity": 144, "temp_c": 4.3, "notes": "loaded reefer truck 7"}'
post /records '{"user_index": 3, "item_id": "LOT-2026-0042", "event_type": "inspection", "quantity": 144, "temp_c": 3.8, "notes": "spot check OK"}'
post /records '{"user_index": 1, "item_id": "LOT-2026-0107", "event_type": "received", "quantity": 36, "temp_c": 9.2, "notes": "TEMP EXCURSION on arrival - flagged"}'

say "Broadcasting event batch"
post /admin/process-batch '{}'

say "Done"
echo "Records confirm in ~75s once mined. Watch the Audit dashboard or:"
echo "  curl $GW/records"
