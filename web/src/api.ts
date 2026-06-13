const BASE = "/api";

export type Worker = {
  user_index: number;
  name: string;
  role: string;
  address: string;
  enrolled: boolean;
  enroll_status: string | null;
};

export type AuditRecord = {
  txid: string;
  block_height: number;
  block_time: string | null;
  worker_name: string | null;
  worker_role: string | null;
  user_index: number | null;
  item_id: string;
  event_type: string;
  quantity: number;
  temp_c: number;
  notes: string;
};

export type InFlight = {
  id: string;
  user_index: number;
  kind: "enroll" | "event";
  payload: Record<string, unknown>;
  status: "pending" | "broadcast";
  txid: string | null;
  created_at: string;
  worker_name: string | null;
};

export type RecordsResponse = {
  confirmed: AuditRecord[];
  in_flight: InFlight[];
};

export type Status = {
  chain_tip: number | null;
  scanned_height: number | null;
  balance_zat: number;
  spendable_zat: number;
  queued_records: number;
  org_address: string;
};

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`${BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (!resp.ok) {
    let detail = resp.statusText;
    try {
      const body = await resp.json();
      detail = body.error ?? JSON.stringify(body);
    } catch {
      /* keep statusText */
    }
    throw new Error(detail);
  }
  return resp.json();
}

export const api = {
  workers: () => request<Worker[]>("/workers"),
  createWorker: (name: string, role: string) =>
    request<{ user_index: number; address: string; submission_id: string }>(
      "/workers",
      { method: "POST", body: JSON.stringify({ name, role }) },
    ),
  records: (filters?: { user_index?: number; event_type?: string; item_id?: string }) => {
    const params = new URLSearchParams();
    if (filters?.user_index !== undefined) params.set("user_index", String(filters.user_index));
    if (filters?.event_type) params.set("event_type", filters.event_type);
    if (filters?.item_id) params.set("item_id", filters.item_id);
    const qs = params.toString();
    return request<RecordsResponse>(`/records${qs ? `?${qs}` : ""}`);
  },
  createRecord: (body: {
    user_index: number;
    item_id: string;
    event_type: string;
    quantity: number;
    temp_c: number;
    notes: string;
  }) =>
    request<{ id: string; status: string }>("/records", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  processBatch: () =>
    request<{ broadcast: { submission_id: string; txid: string }[] }>(
      "/admin/process-batch",
      { method: "POST" },
    ),
  rebuild: () =>
    request<{ rebuilt_records: number }>("/admin/rebuild", { method: "POST" }),
  status: () => request<Status>("/status"),
};

export const explorerUrl = (txid: string) =>
  `https://mainnet.zcashexplorer.app/transactions/${txid}`;
