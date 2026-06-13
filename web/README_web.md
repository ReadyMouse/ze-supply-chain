# web/ — React Demo UI

Vite + React + TypeScript frontend for the ZE Supply Chain hackathon demo. Talks to the gateway via a dev-server proxy.

## Purpose

Demonstrates the end-to-end cold-chain audit flow:

1. **Home** — enroll workers (New User) and log temperature events (Log Temp)
2. **Under the Hood** — shows the exact 512-byte memo hex that will go on-chain
3. **Dashboard** — reconstructed ledger, in-flight queue, batch/rebuild admin actions

## Layout

| Path | Role |
|---|---|
| `src/App.tsx` | Shell layout and Home ↔ Audit page switch |
| `src/Home.tsx` | New User / Log Temp modals |
| `src/Dashboard.tsx` | Audit table, filters, admin buttons |
| `src/RecordDetail.tsx` | Expandable memo annotation panel |
| `src/UnderTheHood.tsx` | Post-submission artifact viewer + explainer |
| `src/api.ts` | Typed gateway client (`/api` → localhost:7700) |
| `src/styles.css` | Dark theme, memo hex colour spans |
| `vite.config.ts` | Dev proxy and React plugin |
| `index.html` | SPA mount point |

## Config files (no inline headers)

- `package.json` — npm deps (React 19, Vite 6)
- `tsconfig.json` — strict TypeScript for React JSX

## Run

```bash
cd web && npm install && npm run dev
# UI at http://localhost:5173 — requires gateway on 7700
```

## Open-source candidacy

**Good candidate** with minor changes. No proprietary dependencies; only assumes the gateway REST API shape documented in `src/api.ts`. Could be repointed to any compatible backend.
