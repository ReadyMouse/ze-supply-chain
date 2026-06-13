// Home Page — Demo Transaction Actions
//
//   Landing page with New User, Log Temp, and Audit Dashboard entry points.
//   Modals submit to gateway and display Under the Hood memo artifacts.
//
// INPUT:
//   - onAudit callback to navigate to Dashboard
//   - api workers/records endpoints
//
// OUTPUT:
//   - Enrollment and event submissions with UnderTheHood artifact display
//
// NOTES:
//   Log Temp requires at least one enrolled worker. Shows live memo hex dumps.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import { useEffect, useState } from "react";
import { api, Worker } from "./api";
import { Artifact, UnderTheHoodSection } from "./UnderTheHood";

type Modal = "none" | "new-user" | "log-temp";

export function Home({ onAudit }: { onAudit: () => void }) {
  const [modal, setModal] = useState<Modal>("none");
  const [artifact, setArtifact] = useState<Artifact | null>(null);

  return (
    <div className="home">
      <p className="tagline">
        Every record below is serialized into the encrypted memo of a shielded
        Zcash transaction and broadcast on-chain. The blockchain is the ledger;
        the database is just a cache.
      </p>

      <p className="buttons-label">Sample Transactions</p>
      <div className="big-buttons">
        <button className="big-btn" onClick={() => setModal("new-user")}>
          <span className="icon">＋</span>
          New User
          <span className="hint">enroll a worker on-chain</span>
        </button>
        <button className="big-btn" onClick={() => setModal("log-temp")}>
          <span className="icon">🌡</span>
          Log Temp
          <span className="hint">record a cold-chain event</span>
        </button>
      </div>
      <button className="audit-btn" onClick={onAudit}>
        <span>⛓</span>
        Audit Dashboard
        <span className="hint">reconstructed ledger + confirmed tx IDs</span>
      </button>

      <UnderTheHoodSection artifact={artifact} />

      {modal === "new-user" && (
        <NewUserModal
          onClose={() => setModal("none")}
          onArtifact={setArtifact}
        />
      )}
      {modal === "log-temp" && (
        <LogTempModal
          onClose={() => setModal("none")}
          onArtifact={setArtifact}
        />
      )}
    </div>
  );
}

function NewUserModal({
  onClose,
  onArtifact,
}: {
  onClose: () => void;
  onArtifact: (a: Artifact) => void;
}) {
  const [name, setName] = useState("");
  const [role, setRole] = useState("warehouse_worker");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ address: string } | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.createWorker(name.trim(), role);
      setResult({ address: r.address });
      onArtifact({ kind: "enroll", workerName: name.trim(), submissionId: r.submission_id, hood: r.under_the_hood });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Enroll New User</h2>
        {error && <div className="error-box">{error}</div>}
        {result ? (
          <>
            <div className="success-box">
              <span className="chip pending">pending</span>
              <p>
                <b>{name}</b> enrolled. Their identity <i>is</i> this shielded
                address, derived via ZIP 32 from the org key:
              </p>
              <span className="mono">{result.address}</span>
              <p className="muted">
                The enrollment memo confirms on-chain in ~75s.
              </p>
            </div>
            <div className="actions">
              <button className="primary" onClick={onClose}>
                Done
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="field">
              <label>Full name</label>
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Alice Nguyen"
                autoFocus
              />
            </div>
            <div className="field">
              <label>Role</label>
              <select value={role} onChange={(e) => setRole(e.target.value)}>
                <option value="warehouse_worker">Warehouse worker</option>
                <option value="driver">Driver</option>
                <option value="inspector">Inspector</option>
                <option value="pharmacist">Pharmacist</option>
              </select>
            </div>
            <div className="actions">
              <button className="ghost" onClick={onClose}>
                Cancel
              </button>
              <button
                className="primary"
                disabled={busy || !name.trim()}
                onClick={submit}
              >
                {busy ? "Enrolling…" : "Enroll"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function LogTempModal({
  onClose,
  onArtifact,
}: {
  onClose: () => void;
  onArtifact: (a: Artifact) => void;
}) {
  const [workers, setWorkers] = useState<Worker[]>([]);
  const [userIndex, setUserIndex] = useState<number | null>(null);
  const [itemId, setItemId] = useState("");
  const [eventType, setEventType] = useState("received");
  const [quantity, setQuantity] = useState(1);
  const [tempC, setTempC] = useState("4.0");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState(false);

  useEffect(() => {
    api
      .workers()
      .then((ws) => {
        setWorkers(ws);
        if (ws.length > 0) setUserIndex(ws[0].user_index);
      })
      .catch((e) => setError((e as Error).message));
  }, []);

  const submit = async () => {
    if (userIndex === null) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.createRecord({
        user_index: userIndex,
        item_id: itemId.trim(),
        event_type: eventType,
        quantity,
        temp_c: parseFloat(tempC),
        notes: notes.trim(),
      });
      const worker = workers.find((w) => w.user_index === userIndex);
      onArtifact({
        kind: "event",
        workerName: worker?.name ?? `#${userIndex}`,
        submissionId: r.id,
        hood: r.under_the_hood,
      });
      setSubmitted(true);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Log Cold-Chain Event</h2>
        {error && <div className="error-box">{error}</div>}
        {submitted ? (
          <>
            <div className="success-box">
              <span className="chip pending">pending</span>
              <p>
                Record queued. It will ride the next batch transaction and
                confirm on-chain in a block or two — watch it move through
                pending → broadcast → confirmed in the Audit view.
              </p>
            </div>
            <div className="actions">
              <button className="primary" onClick={onClose}>
                Done
              </button>
            </div>
          </>
        ) : workers.length === 0 ? (
          <>
            <p className="muted">
              No enrolled workers yet — create one with New User first.
            </p>
            <div className="actions">
              <button className="ghost" onClick={onClose}>
                Close
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="field">
              <label>Acting as</label>
              <select
                value={userIndex ?? ""}
                onChange={(e) => setUserIndex(Number(e.target.value))}
              >
                {workers.map((w) => (
                  <option key={w.user_index} value={w.user_index}>
                    {w.name} ({w.role})
                  </option>
                ))}
              </select>
            </div>
            <div className="field">
              <label>Item / lot ID</label>
              <input
                value={itemId}
                onChange={(e) => setItemId(e.target.value)}
                placeholder="LOT-2026-0042"
              />
            </div>
            <div className="row">
              <div className="field">
                <label>Event</label>
                <select
                  value={eventType}
                  onChange={(e) => setEventType(e.target.value)}
                >
                  <option value="received">Received</option>
                  <option value="handoff">Handoff</option>
                  <option value="inspection">Inspection</option>
                </select>
              </div>
              <div className="field">
                <label>Quantity</label>
                <input
                  type="number"
                  min={1}
                  value={quantity}
                  onChange={(e) => setQuantity(Number(e.target.value))}
                />
              </div>
              <div className="field">
                <label>Temp °C</label>
                <input
                  type="number"
                  step="0.1"
                  value={tempC}
                  onChange={(e) => setTempC(e.target.value)}
                />
              </div>
            </div>
            <div className="field">
              <label>Notes (max 350 bytes — it has to fit in the memo)</label>
              <textarea
                rows={2}
                maxLength={350}
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder="seal intact, reefer at temp"
              />
            </div>
            <div className="actions">
              <button className="ghost" onClick={onClose}>
                Cancel
              </button>
              <button
                className="primary"
                disabled={busy || !itemId.trim() || userIndex === null}
                onClick={submit}
              >
                {busy ? "Submitting…" : "Submit Record"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
