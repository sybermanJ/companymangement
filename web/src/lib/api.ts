// Thin JSON client. Every write sends X-Requested-With (server CSRF guard).

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message)
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {}
  if (method !== 'GET') headers['X-Requested-With'] = 'cm'
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  const res = await fetch(`/api/v1${path}`, {
    method,
    headers,
    credentials: 'same-origin',
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (res.status === 204) return undefined as T
  const data = await res.json().catch(() => ({}))
  if (!res.ok) throw new ApiError(res.status, data.code ?? 'error', data.message ?? res.statusText)
  return data as T
}

export const api = {
  get: <T>(p: string) => request<T>('GET', p),
  post: <T>(p: string, b?: unknown) => request<T>('POST', p, b ?? {}),
  put: <T>(p: string, b?: unknown) => request<T>('PUT', p, b ?? {}),
  patch: <T>(p: string, b?: unknown) => request<T>('PATCH', p, b ?? {}),
  del: <T>(p: string) => request<T>('DELETE', p),
}

export interface Membership { tenant_id: string; tenant_name: string; is_admin: boolean; is_finance: boolean; can_view_contracts?: boolean; currency?: string }
export interface Me { user_id: string; tenant_id: string; email: string; name: string; email_verified: boolean; memberships: Membership[] }
export interface Department { id: string; parent_id: string | null; name: string; head_user_id: string | null }
export interface Member { user_id: string; email: string; display_name: string; status: string; is_admin: boolean; is_finance: boolean; can_view_contracts?: boolean }
export interface Role { id: string; department_id: string | null; name: string }
export interface Assignment { user_id: string; role_id: string; role_name: string; department_id: string; department_name: string }

export interface Project {
  id: string; key: string; name: string; owner_id: string; owner_name: string; department_id: string | null; status: string
  planned_start: string | null; planned_end: string | null; currency: string; labour_budget_minor?: number | null
  estimate_minutes: number; logged_minutes: number; open_items: number; total_items: number
  budget_used_pct: number | null; open_alerts: number; cost?: ProjectCost
}
export interface ProjectCost {
  currency: string; budget_minor: number | null; spent_minor: number; forecast_minor: number; used_pct: number | null
  costed_minutes: number; uncosted_entries: number; remaining_minutes: number
  expense_minor?: number; expense_budget_minor?: number | null; expense_in_approval_minor?: number; total_minor?: number; total_budget_minor?: number | null
}
export interface Item {
  id: string; project_id: string | null; parent_id: string | null; kind: 'phase' | 'task' | 'milestone' | 'routine' | 'request'
  key: string | null; title: string; description: string; status: string; holder_user_id: string | null; holder_name: string | null
  planned_start: string | null; planned_end: string | null; estimate_minutes: number | null; progress_pct: number
  sort_order: number; version: number; logged_minutes: number
  target_role_id: string | null; target_department_id: string | null; routine_template_id: string | null; occurrence_date: string | null
  requested_by: string | null; handoff_count: number; open_handoff: 'pending_approval' | 'pending_accept' | null
}
export interface Spec { id: string; kind: string; min_count: number; required: boolean; instructions: string; submitted: number }
export interface Submission { id: string; spec_id: string; work_log_id: string | null; attachment_id: string | null; value: string; submitted_by_name: string; created_at: string }
export interface Handoff {
  id: string; work_item_id: string; item_key: string | null; item_title: string; project_id: string | null
  from_user: string; from_name: string; to_user: string; to_name: string; requested_by: string; reason: string; note: string
  new_due: string | null; needs_approval: boolean; state: string; decision_note: string; is_return: boolean; created_at: string; decided_at: string | null
}
export interface Commit { id: string; repo_name: string | null; sha: string; author_name: string; user_name: string | null; message: string; url: string; committed_at: string | null }
export interface Notification { id: string; kind: string; subject: string; body: string; link: string; read_at: string | null; created_at: string }
export interface ItemDetail {
  item: Item; logs: WorkLog[]; attachments: Attachment[]; events: ItemEvent[]; can_manage: boolean
  specs: Spec[]; submissions: Submission[]; handoffs: Handoff[]; commits: Commit[]; can_reassign: boolean; can_return: boolean
}
export interface Dependency { from_item: string; to_item: string; kind: string; lag_days: number }
export interface WorkLog { id: string; work_item_id: string; user_id: string; user_name: string; started_at: string; minutes: number; description: string; source: string; locked: boolean }
export interface Attachment { id: string; work_item_id: string; work_log_id: string | null; kind: string; file_name: string; mime: string; size_bytes: number; created_at: string }
export interface ItemEvent { id: string; actor_id: string; actor_name: string; kind: string; payload: Record<string, unknown>; created_at: string }
export interface Timer { work_item_id: string; started_at: string }

/** Upload files to an item (optionally tied to a work log). */
export async function uploadFiles(itemId: string, files: File[], workLogId?: string, specId?: string): Promise<Attachment[]> {
  const form = new FormData()
  if (workLogId) form.append('work_log_id', workLogId)
  if (specId) form.append('spec_id', specId)
  for (const f of files) form.append('file', f, f.name)
  const res = await fetch(`/api/v1/items/${itemId}/attachments`, { method: 'POST', body: form, headers: { 'X-Requested-With': 'cm' }, credentials: 'same-origin' })
  const data = await res.json().catch(() => ({}))
  if (!res.ok) throw new ApiError(res.status, data.code ?? 'error', data.message ?? res.statusText)
  return data as Attachment[]
}
