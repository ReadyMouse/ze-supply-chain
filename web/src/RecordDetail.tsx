// Record Detail Panel — Annotated Memo Viewer
//
//   Expandable row showing payment JSON, derivation paths, and colour-coded
//   hex dump of the 512-byte memo for a confirmed audit record or in-flight item.
//
// INPUT:
//   - AuditRecord or InFlight submission from Dashboard/Home
//   - api.annotate for re-encoding stored fields
//
// OUTPUT:
//   - Rendered UnderTheHood panel with memo_spans hex visualization
//
// NOTES:
//   Annotated spans come from memo-schema encode_memo_annotated via gateway.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import { useEffect, useState } from "react";
import { api, AuditRecord, explorerUrl, InFlight, MemoSpan, UnderTheHood, Worker } from "./api";

const PALETTE = [
  "span-c0","span-c1","span-c2","span-c3",
  "span-c4","span-c5","span-c6","span-c7",
];

export function RecordDetail({ record }: { record: AuditRecord }) {
  const [detail, setDetail] = useState<UnderTheHood | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .annotate(record)
      .then((d) => setDetail(d as unknown as UnderTheHood))
      .catch((e) => setError((e as Error).message));
  }, [record.txid, record.output_pool, record.output_index]);

  const colSpan = 10;

  return (
    <tr className="detail-row">
      <td colSpan={colSpan}>
        <div className="detail-panel">
          {error && <p className="muted" style={{ color: "var(--red)" }}>{error}</p>}
          {!detail && !error && <p className="muted">Loading…</p>}
          {detail && (
            <div className="detail-inner">
              <div className="detail-cols">
                <div className="detail-col">
                  <h5>Payment request sent to wallet service</h5>
                  <p className="muted detail-sub">
                    The JSON constructed for this record. The wallet service
                    proposes a transaction from this, which lightwalletd
                    broadcasts to the network.
                  </p>
                  <pre className="json">{JSON.stringify(detail.payment_json, null, 2)}</pre>
                  <div className="kv" style={{ marginTop: "0.6rem" }}>
                    <span>sender</span>
                    <span>
                      <b>{detail.sender.label}</b>{" "}
                      <code>{detail.sender.derivation_path}</code>
                    </span>
                  </div>
                  <div className="kv">
                    <span>receiver</span>
                    <span>
                      <b>{detail.receiver.label}</b>{" "}
                      <code>{detail.receiver.derivation_path}</code>
                      <br />
                      <code className="wrap">{detail.receiver.address}</code>
                    </span>
                  </div>
                </div>

                <div className="detail-col">
                  <h5>512-byte memo field — encrypted on-chain</h5>
                  <p className="muted detail-sub">
                    The exact binary buffer that rode inside the shielded output.
                    Decryptable only with the org viewing key.
                  </p>
                  <HexDump hex={detail.memo_hex} spans={detail.memo_spans} />
                  <div className="legend" style={{ marginTop: "0.6rem" }}>
                    {detail.memo_spans.map((s, i) => (
                      <span key={i} className="legend-item">
                        <i className={`swatch ${PALETTE[i % PALETTE.length]}`} />
                        <code>{s.start}–{s.end - 1}</code> {s.label}
                      </span>
                    ))}
                  </div>
                </div>
              </div>

              <div className="detail-proof">
                <span className="muted" style={{ fontSize: "0.78rem" }}>on-chain proof</span>
                <a
                  className="txlink"
                  href={explorerUrl(record.txid)}
                  target="_blank"
                  rel="noreferrer"
                >
                  {record.txid}
                </a>
              </div>
            </div>
          )}
        </div>
      </td>
    </tr>
  );
}

function HexDump({ hex, spans }: { hex: string; spans: MemoSpan[] }) {
  const bytes = hexToBytes(hex);
  const classFor = (i: number) => {
    const idx = spans.findIndex((s) => i >= s.start && i < s.end);
    if (idx === -1) return "";
    return spans[idx].label.startsWith("zero padding")
      ? "span-pad"
      : PALETTE[idx % PALETTE.length];
  };
  const paddingStart = spans.find((s) => s.label.startsWith("zero padding"))?.start;
  const rows = [];
  let collapsed = false;
  for (let off = 0; off < bytes.length; off += 16) {
    if (paddingStart !== undefined && off > paddingStart && off + 16 <= bytes.length) {
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
  for (let i = 0; i < out.length; i++)
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

// ---------- in-flight (queued) detail ----------

export function InFlightDetail({
  submission,
  workers,
}: {
  submission: InFlight;
  workers: Worker[];
}) {
  const [detail, setDetail] = useState<UnderTheHood | null>(null);
  const [error, setError] = useState<string | null>(null);

  const worker = workers.find((w) => w.user_index === submission.user_index);

  useEffect(() => {
    const p = submission.payload as Record<string, unknown>;
    const isEnroll = submission.kind === "enroll";

    const body: Record<string, unknown> = {
      user_index: submission.user_index,
      address: worker?.address,
      worker_name: submission.worker_name ?? worker?.name,
      worker_role: worker?.role,
    };

    if (!isEnroll) {
      body.item_id = p.item_id;
      body.event_type = (p as { Event?: { event_type?: string } }).Event?.event_type
        ?? p.event_type;
      const event = (p as { Event?: Record<string, unknown> }).Event ?? p;
      body.quantity = event.quantity as number;
      body.temp_centi = event.temp_centi as number;
      body.client_ts = event.client_ts as number;
      body.notes = event.notes as string;
    } else {
      const enroll = (p as { Enroll?: Record<string, unknown> }).Enroll ?? p;
      body.worker_name = enroll.name as string ?? body.worker_name;
      body.worker_role = enroll.role as string ?? body.worker_role;
    }

    api
      .annotate(body as Parameters<typeof api.annotate>[0])
      .then((d) => setDetail(d as unknown as UnderTheHood))
      .catch((e) => setError((e as Error).message));
  }, [submission.id]);

  return (
    <tr className="detail-row">
      <td colSpan={6}>
        <div className="detail-panel">
          {error && <p className="muted" style={{ color: "var(--red)" }}>{error}</p>}
          {!detail && !error && <p className="muted">Loading…</p>}
          {detail && (
            <div className="detail-inner">
              <div className="detail-cols">
                <div className="detail-col">
                  <h5>Queued payload — not yet broadcast</h5>
                  <p className="muted detail-sub">
                    This record is sitting in the batch queue. When the batch is
                    processed, this payload becomes a shielded output in a
                    send-many transaction.
                  </p>
                  <pre className="json">{JSON.stringify(submission.payload, null, 2)}</pre>
                  <div className="kv" style={{ marginTop: "0.6rem" }}>
                    <span>sender</span>
                    <span>
                      <b>{detail.sender.label}</b>{" "}
                      <code>{detail.sender.derivation_path}</code>
                    </span>
                  </div>
                  <div className="kv">
                    <span>receiver</span>
                    <span>
                      <b>{detail.receiver.label}</b>{" "}
                      <code>{detail.receiver.derivation_path}</code>
                      {detail.receiver.address && (
                        <>
                          <br />
                          <code className="wrap">{detail.receiver.address}</code>
                        </>
                      )}
                    </span>
                  </div>
                </div>

                <div className="detail-col">
                  <h5>512-byte memo to be encrypted on-chain</h5>
                  <p className="muted detail-sub">
                    Exactly these bytes will ride inside the shielded output
                    once the batch transaction is broadcast and mined.
                  </p>
                  <HexDump hex={detail.memo_hex} spans={detail.memo_spans} />
                  <div className="legend" style={{ marginTop: "0.6rem" }}>
                    {detail.memo_spans.map((s, i) => (
                      <span key={i} className="legend-item">
                        <i className={`swatch ${PALETTE[i % PALETTE.length]}`} />
                        <code>{s.start}–{s.end - 1}</code> {s.label}
                      </span>
                    ))}
                  </div>
                </div>
              </div>

              <div className="detail-proof">
                <span className="muted" style={{ fontSize: "0.78rem" }}>
                  on-chain proof — available after batch is processed
                </span>
                <span className="muted" style={{ fontSize: "0.8rem" }}>
                  tx hash will appear here once broadcast and confirmed
                </span>
              </div>
            </div>
          )}
        </div>
      </td>
    </tr>
  );
}
