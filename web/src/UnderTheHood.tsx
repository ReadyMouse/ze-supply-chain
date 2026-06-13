// Under the Hood — Memo Construction Explainer
//
//   Educational UI explaining ZIP 32 identity, memo encoding, and audit rebuild.
//   Renders post-submission artifacts with annotated hex dumps and tx polling.
//
// INPUT:
//   - Artifact from Home modals (enroll or event UnderTheHood payload)
//   - api.submission for pending → confirmed status polling
//
// OUTPUT:
//   - Explainer copy, ArtifactView with memo hex, block explorer links
//
// NOTES:
//   Designed for demo/judge visibility into exactly what goes on-chain.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import { ReactElement, useEffect, useState } from "react";
import { api, explorerUrl, Submission, UnderTheHood as Hood } from "./api";

export type Artifact = {
  kind: "enroll" | "event";
  workerName: string;
  submissionId: string;
  hood: Hood;
};

const PALETTE = [
  "span-c0",
  "span-c1",
  "span-c2",
  "span-c3",
  "span-c4",
  "span-c5",
  "span-c6",
  "span-c7",
];

export function UnderTheHoodSection({ artifact }: { artifact: Artifact | null }) {
  return (
    <section className="hood">
      <hr className="divider" />
      <h2 className="hood-title">Under the Hood</h2>

      <div className="hood-explainer">
        <div>
          <b>[New User]</b> derives a fresh shielded address for the worker from
          the org master key via ZIP 32 hierarchical derivation — the address{" "}
          <i>is</i> their identity, no account database required. An enrollment
          record (name + role) is encoded into a 512-byte memo and queued.
        </div>
        <div>
          <b>[Log Temp]</b> serializes the event (item, type, quantity,
          temperature, notes) into a compact MessagePack memo addressed to the
          submitting worker's z-address, and queues it for the next batch
          transaction.
        </div>
        <div>
          <b>[Audit]</b> reads nothing from this app's memory — it queries a
          Postgres reconstruction built by scanning the blockchain and
          trial-decrypting memos with the org viewing key. Wipe it and it
          rebuilds from chain.
        </div>
      </div>

      {artifact ? (
        <ArtifactView artifact={artifact} />
      ) : (
        <p className="hood-hint muted">
          Press <b>New User</b> or <b>Log Temp</b> and the actual constructed
          payload — the real bytes headed for the blockchain — will appear here,
          annotated.
        </p>
      )}
    </section>
  );
}

function ArtifactView({ artifact }: { artifact: Artifact }) {
  const { hood, kind, workerName, submissionId } = artifact;
  const bytes = hexToBytes(hood.memo_hex);
  const { sub, refresh } = useSubmissionStatus(submissionId);

  return (
    <div className="artifact">
      <h3>
        {kind === "enroll"
          ? `Enrollment transaction for ${workerName} — constructed live`
          : `Event record for ${workerName} — constructed live`}
      </h3>

      <div className="artifact-block">
        <h4>1 · Identity via key derivation</h4>
        <p className="muted">
          {kind === "enroll"
            ? "A unique shielded address, deterministically derived from the org master seed for this user index. The address is stored in the database, but it can also be re-derived at any time from seed + index alone — so if the database were wiped, every address and its full history would be recoverable from the blockchain."
            : "The worker's derived address and submitter_index in the memo attribute this record. The shielded output itself lands at the org receive address (treasury pool), not the worker's personal address."}
        </p>
        {kind === "event" && hood.submitter ? (
          <>
            <div className="kv">
              <span>submitter path</span>
              <code>{hood.submitter.derivation_path}</code>
            </div>
            <div className="kv">
              <span>submitter address</span>
              <code className="wrap">{hood.submitter.address}</code>
            </div>
          </>
        ) : (
          <>
            <div className="kv">
              <span>ZIP 32 path</span>
              <code>{hood.derivation_path}</code>
            </div>
            <div className="kv">
              <span>shielded address</span>
              <code className="wrap">{hood.address}</code>
            </div>
          </>
        )}
      </div>

      <div className="artifact-block">
        <h4>2 · Payment request, as sent to the wallet service → lightwalletd</h4>
        <p className="muted">
          The JSON constructed for this record before transmission. The{" "}
          <code>memo_plaintext</code> is what gets encoded into the 512-byte
          field below; on-chain it exists only in encrypted form.
        </p>
        <pre className="json">{JSON.stringify(hood.payment_json, null, 2)}</pre>
        <div className="kv">
          <span>sender</span>
          <span>
            <b>{hood.sender.label}</b>{" "}
            <code>{hood.sender.derivation_path}</code> — the org wallet that
            signs and pays fees
          </span>
        </div>
        <div className="kv">
          <span>receiver</span>
          <span>
            <b>
              {hood.receiver.label} ({hood.receiver.role})
            </b>{" "}
            <code>{hood.receiver.derivation_path}</code> —
            {kind === "event"
              ? " org treasury pool; memo submitter_index names the worker"
              : ` the worker's derived address. The indexer's address book maps address → identity, so receipt here registers ${hood.receiver.label}.`}
          </span>
        </div>
      </div>

      <div className="artifact-block">
        <h4>3 · The 512-byte memo field, byte for byte</h4>
        <p className="muted">
          This exact buffer is encrypted into a shielded output. Only holders of
          the org viewing key can ever read it back.
        </p>
        <HexDump bytes={bytes} spans={hood.memo_spans} />
        <div className="legend">
          {hood.memo_spans.map((s, i) => (
            <span key={i} className="legend-item">
              <i className={`swatch ${PALETTE[i % PALETTE.length]}`} />
              <code>
                {s.start}–{s.end - 1}
              </code>{" "}
              {s.label}
            </span>
          ))}
        </div>
      </div>

      <div className="artifact-block">
        <h4>4 · Transaction plan → on-chain proof</h4>
        <div className="txplan">
          <div className="txplan-row">
            <span className="txplan-tag in">inputs</span>
            <span>
              shielded note(s) from the org treasury (<code>m/32'/133'/0'</code>
              ), selected when the batch is processed
            </span>
          </div>
          <div className="txplan-row">
            <span className="txplan-tag out">output</span>
            <span>
              0.0001 ZEC → <code className="wrap">{shorten(hood.address)}</code>{" "}
              carrying the encrypted memo above
              {kind === "event" ? " (one output per record in the batch)" : ""}
            </span>
          </div>
          <div className="txplan-row">
            <span className="txplan-tag out">change</span>
            <span>
              like paying with a $20 bill: the treasury spends a whole note, the
              difference comes back to it as a fresh shielded note
            </span>
          </div>
          <div className="txplan-row">
            <span className="txplan-tag fee">fee</span>
            <span>
              ZIP 317: 5,000 zatoshis × logical actions — about $0.01 per batch
            </span>
          </div>
        </div>
        <p className="muted">
          On-chain observers see only that <i>a</i> shielded transaction
          happened — amounts, addresses, and memo contents are all encrypted.
        </p>
      </div>

      <div className="proof-box">
        <div className="proof-header">
          <span className="proof-title">On-chain proof</span>
          <button className="ghost proof-refresh" onClick={refresh}>
            ↻ Refresh
          </button>
        </div>
        {sub?.txid ? (
          <div className="proof-confirmed">
            <span className={`chip ${sub.status}`}>{sub.status}</span>
            <div className="proof-hash">
              <span className="muted">tx hash</span>
              <a
                className="txlink proof-txlink"
                href={explorerUrl(sub.txid)}
                target="_blank"
                rel="noreferrer"
              >
                {sub.txid}
              </a>
            </div>
            <p className="muted proof-note">
              This transaction is permanently recorded on the Zcash mainnet
              blockchain. Anyone can verify it independently using the link
              above — no trust in this application required.
            </p>
          </div>
        ) : (
          <div className="proof-pending">
            <span className="chip pending">pending</span>
            <p className="muted">
              The record is queued. Press{" "}
              <b>⚡ Process Batch</b> in the Audit view to broadcast immediately,
              or wait for the automatic batch timer. The tx hash will appear
              here — hit Refresh or wait for the auto-poll.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

/// Poll the submission automatically, and expose a manual refresh.
function useSubmissionStatus(id: string): {
  sub: Submission | null;
  refresh: () => void;
} {
  const [sub, setSub] = useState<Submission | null>(null);
  const [tick, setTick] = useState(0);

  const refresh = () => setTick((n) => n + 1);

  useEffect(() => {
    let stop = false;
    const poll = async () => {
      try {
        const s = await api.submission(id);
        if (!stop) setSub(s);
        if (s.status === "confirmed") return true;
      } catch {
        /* keep polling */
      }
      return false;
    };
    poll();
    const t = setInterval(async () => {
      if (await poll()) clearInterval(t);
    }, 4000);
    return () => {
      stop = true;
      clearInterval(t);
    };
    // tick causes a re-run on manual refresh
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, tick]);

  return { sub, refresh };
}

function HexDump({
  bytes,
  spans,
}: {
  bytes: Uint8Array;
  spans: { label: string; start: number; end: number }[];
}) {
  const classFor = (i: number): string => {
    const idx = spans.findIndex((s) => i >= s.start && i < s.end);
    if (idx === -1) return "";
    const isPadding = spans[idx].label.startsWith("zero padding");
    return isPadding ? "span-pad" : PALETTE[idx % PALETTE.length];
  };

  const paddingStart = spans.find((s) => s.label.startsWith("zero padding"))?.start;
  const rows: ReactElement[] = [];
  let collapsed = false;

  for (let off = 0; off < bytes.length; off += 16) {
    // Collapse rows that are pure padding (keep the first one).
    if (
      paddingStart !== undefined &&
      off > paddingStart &&
      off + 16 <= bytes.length
    ) {
      if (!collapsed) {
        rows.push(
          <div className="hex-row muted" key="ellipsis">
            <span className="hex-off">⋮</span>
            <span>{bytes.length - off} more zero bytes of padding</span>
          </div>,
        );
        collapsed = true;
      }
      continue;
    }
    rows.push(
      <div className="hex-row" key={off}>
        <span className="hex-off">{off.toString(16).padStart(3, "0")}</span>
        <span className="hex-bytes">
          {Array.from(bytes.slice(off, off + 16)).map((b, i) => (
            <span key={i} className={`hex-byte ${classFor(off + i)}`}>
              {b.toString(16).padStart(2, "0")}
            </span>
          ))}
        </span>
      </div>,
    );
  }

  return <div className="hexdump">{rows}</div>;
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function shorten(addr: string): string {
  return addr.length > 28 ? `${addr.slice(0, 16)}…${addr.slice(-8)}` : addr;
}
