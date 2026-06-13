import { useCallback, useEffect, useState } from "react";
import {
  api,
  AuditRecord,
  explorerUrl,
  InFlight,
  Status,
  Worker,
} from "./api";

const POLL_MS = 5000;

export function Dashboard() {
  const [status, setStatus] = useState<Status | null>(null);
  const [records, setRecords] = useState<AuditRecord[]>([]);
  const [inFlight, setInFlight] = useState<InFlight[]>([]);
  const [workers, setWorkers] = useState<Worker[]>([]);
  const [filterWorker, setFilterWorker] = useState<string>("");
  const [filterEvent, setFilterEvent] = useState<string>("");
  const [filterItem, setFilterItem] = useState<string>("");
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [s, r, w] = await Promise.all([
        api.status(),
        api.records({
          user_index: filterWorker ? Number(filterWorker) : undefined,
          event_type: filterEvent || undefined,
          item_id: filterItem || undefined,
        }),
        api.workers(),
      ]);
      setStatus(s);
      setRecords(r.confirmed);
      setInFlight(r.in_flight);
      setWorkers(w);
    } catch {
      /* transient poll errors are fine */
    }
  }, [filterWorker, filterEvent, filterItem]);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, POLL_MS);
    return () => clearInterval(t);
  }, [refresh]);

  const processBatch = async () => {
    setBusy("batch");
    setNotice(null);
    try {
      const r = await api.processBatch();
      setNotice(
        r.broadcast.length > 0
          ? `Broadcast ${r.broadcast.length} record(s) in tx ${r.broadcast[0].txid.slice(0, 16)}…`
          : "Queue was empty — nothing to broadcast.",
      );
      refresh();
    } catch (e) {
      setNotice(`Batch failed: ${(e as Error).message}`);
    } finally {
      setBusy(null);
    }
  };

  const rebuild = async () => {
    setBusy("rebuild");
    setNotice(null);
    try {
      const r = await api.rebuild();
      setNotice(
        `Audit tables wiped and rebuilt from chain: ${r.rebuilt_records} records reconstructed from encrypted memos.`,
      );
      refresh();
    } catch (e) {
      setNotice(`Rebuild failed: ${(e as Error).message}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div>
      <div className="statusbar">
        <span>
          chain tip <b>{status?.chain_tip ?? "—"}</b>
        </span>
        <span>
          scanned <b>{status?.scanned_height ?? "—"}</b>
        </span>
        <span>
          balance <b>{status ? zec(status.balance_zat) : "—"}</b>
        </span>
        <span>
          spendable <b>{status ? zec(status.spendable_zat) : "—"}</b>
        </span>
        <span>
          batch queue <b>{status?.queued_records ?? "—"}</b>
        </span>
      </div>

      <div className="toolbar">
        <select
          value={filterWorker}
          onChange={(e) => setFilterWorker(e.target.value)}
        >
          <option value="">All workers</option>
          {workers.map((w) => (
            <option key={w.user_index} value={w.user_index}>
              {w.name}
            </option>
          ))}
        </select>
        <select
          value={filterEvent}
          onChange={(e) => setFilterEvent(e.target.value)}
        >
          <option value="">All events</option>
          <option value="received">Received</option>
          <option value="handoff">Handoff</option>
          <option value="inspection">Inspection</option>
        </select>
        <input
          placeholder="Filter by item ID…"
          value={filterItem}
          onChange={(e) => setFilterItem(e.target.value)}
        />
        <div className="spacer" />
        <button
          className="ghost"
          disabled={busy !== null}
          onClick={processBatch}
        >
          {busy === "batch" ? "Broadcasting…" : "⚡ Process Batch"}
        </button>
        <button className="ghost" disabled={busy !== null} onClick={rebuild}>
          {busy === "rebuild" ? "Rebuilding…" : "♻ Rebuild from Chain"}
        </button>
      </div>

      {notice && <div className="success-box">{notice}</div>}

      {inFlight.length > 0 && (
        <>
          <div className="section-label">In flight</div>
          <table className="records">
            <thead>
              <tr>
                <th>Status</th>
                <th>Kind</th>
                <th>Worker</th>
                <th>Detail</th>
                <th>Tx</th>
                <th>Submitted</th>
              </tr>
            </thead>
            <tbody>
              {inFlight.map((s) => (
                <tr key={s.id}>
                  <td>
                    <span className={`chip ${s.status}`}>{s.status}</span>
                  </td>
                  <td>{s.kind}</td>
                  <td>{s.worker_name ?? `#${s.user_index}`}</td>
                  <td className="muted">{inFlightDetail(s)}</td>
                  <td>
                    {s.txid ? (
                      <a
                        className="txlink"
                        href={explorerUrl(s.txid)}
                        target="_blank"
                        rel="noreferrer"
                      >
                        {s.txid.slice(0, 12)}…
                      </a>
                    ) : (
                      <span className="muted">awaiting batch</span>
                    )}
                  </td>
                  <td className="muted">{fmtTime(s.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      <div className="section-label">Confirmed on-chain</div>
      {records.length === 0 ? (
        <div className="empty">
          No confirmed records yet. Submit one, process the batch, and wait a
          block (~75s).
        </div>
      ) : (
        <table className="records">
          <thead>
            <tr>
              <th>Status</th>
              <th>Block</th>
              <th>Time</th>
              <th>Worker</th>
              <th>Item</th>
              <th>Event</th>
              <th>Qty</th>
              <th>Temp</th>
              <th>Notes</th>
              <th>Tx (verify independently)</th>
            </tr>
          </thead>
          <tbody>
            {records.map((r) => (
              <tr key={`${r.txid}-${r.item_id}-${r.block_height}-${r.user_index}`}>
                <td>
                  <span className="chip confirmed">confirmed</span>
                </td>
                <td className="mono">{r.block_height}</td>
                <td className="muted">{r.block_time ? fmtTime(r.block_time) : "—"}</td>
                <td>
                  {r.worker_name ?? "—"}
                  {r.worker_role && (
                    <div className="muted" style={{ fontSize: "0.72rem" }}>
                      {r.worker_role}
                    </div>
                  )}
                </td>
                <td className="mono">{r.item_id}</td>
                <td>{r.event_type}</td>
                <td>{r.quantity}</td>
                <td className={r.temp_c > 8 || r.temp_c < 2 ? "temp-bad" : ""}>
                  {r.temp_c.toFixed(1)}°C
                </td>
                <td className="muted">{r.notes || "—"}</td>
                <td>
                  <a
                    className="txlink"
                    href={explorerUrl(r.txid)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {r.txid.slice(0, 12)}…
                  </a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function zec(zat: number): string {
  return `${(zat / 1e8).toFixed(5)} ZEC`;
}

function fmtTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function inFlightDetail(s: InFlight): string {
  if (s.kind === "enroll") {
    const p = s.payload as { name?: string; role?: string };
    return `enroll ${p.name ?? ""} (${p.role ?? ""})`;
  }
  const p = s.payload as {
    item_id?: string;
    event_type?: string;
    temp_centi?: number;
  };
  const temp =
    p.temp_centi !== undefined ? ` @ ${(p.temp_centi / 100).toFixed(1)}°C` : "";
  return `${p.event_type ?? "event"} ${p.item_id ?? ""}${temp}`;
}
