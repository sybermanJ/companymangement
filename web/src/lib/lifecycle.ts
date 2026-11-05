// Types for requests (claims), approvals, contracts and the project file.

export const CLAIM_KINDS = ['reimbursement', 'advance', 'payment', 'purchase', 'trip'] as const
export const LINE_CATEGORIES = ['flight', 'train', 'hotel', 'meals', 'local_transport', 'allowance', 'equipment', 'materials', 'software', 'outsourcing', 'entertainment', 'other'] as const
export const CONTRACT_KINDS = ['sales', 'purchase', 'outsourcing', 'nda', 'framework', 'other'] as const
export const STEP_KINDS = ['phase_manager', 'project_manager', 'department_head', 'finance', 'admin', 'user'] as const
export const ROUTE_SUBJECTS = [...CLAIM_KINDS, 'contract'] as const

export interface Claim {
  id: string; key: string; kind: string; project_id: string | null; project_key: string | null; project_name: string | null
  project_currency: string | null; phase_id: string | null; phase_title: string | null; parent_id: string | null; parent_key: string | null
  applicant_id: string; applicant_name: string; title: string; justification: string; currency: string; amount_minor: number
  payee: string; destination: string; depart_on: string | null; return_on: string | null; travellers: string[]; status: string
  approval_id: string | null; paid_at: string | null; payment_ref: string; project_amount_minor: number | null
  submitted_at: string | null; created_at: string; updated_at: string; waiting_for: string[]
}
export interface Line { id?: string; spent_on: string | null; category: string; description: string; amount_minor: number }
export interface Doc { id: string; label: string; version: number; file_name: string; mime: string; size_bytes: number; uploaded_by_name: string; created_at: string }
export interface Action { id: string; step: number | null; actor_name: string; action: string; to_name: string | null; note: string; created_at: string }
export interface ApprovalView {
  id: string; state: string; current_step: number; round: number
  steps: { index: number; kind: string; label: string; users: { id: string; name: string }[]; skipped: boolean }[]
}
export interface Contract {
  id: string; key: string; project_id: string | null; project_key: string | null; project_name: string | null; title: string
  counterparty: string; kind: string; currency: string; amount_minor: number; owner_id: string; owner_name: string; status: string
  signed_on: string | null; starts_on: string | null; ends_on: string | null; summary: string; approval_id: string | null
  created_by: string; created_at: string; updated_at: string; waiting_for: string[]; settled_minor: number
}
export interface Milestone { id: string; phase_id: string | null; phase_title: string | null; title: string; due_on: string | null; amount_minor: number; status: string; settled_on: string | null }
export interface Route { id: string; subject: string; name: string; min_amount_minor: number; steps: { kind: string; user_id?: string | null; label?: string }[]; active: boolean }
export interface Expenses {
  currency: string; total_minor: number; pending_fx: number; in_approval_minor: number; by_category: [string, number][]
  by_phase: { phase_id: string; title: string; budget_minor: number | null; spent_minor: number; logged_minutes: number; manager: string | null }[]
  budget_minor: number | null; advances_outstanding_minor: number
}
export interface TimelineEntry {
  at: string; source: 'work' | 'log' | 'claim' | 'contract'; who: string; kind: string; ref: string | null; title: string
  id?: string; note?: string; to?: string | null; minutes?: number; payload?: Record<string, unknown>
  amount_minor?: number; currency?: string; justification?: string; phase?: string | null; claim_kind?: string; contract_kind?: string; counterparty?: string
}

export const STATUS_TONE: Record<string, string> = {
  draft: '', in_approval: 'level-warn', returned: 'level-critical', rejected: 'level-critical', approved: 'level-info',
  paid: 'level-info', closed: '', cancelled: '', signed: 'level-info', completed: '', terminated: 'level-critical',
}

export async function uploadDocs(owner: string, label: string, files: FileList | File[]): Promise<void> {
  const fd = new FormData()
  fd.append('label', label)
  for (const f of Array.from(files)) fd.append('file', f)
  const res = await fetch(`/api/v1/${owner}/documents`, { method: 'POST', body: fd, headers: { 'X-Requested-With': 'cm' }, credentials: 'same-origin' })
  if (!res.ok) {
    const d = await res.json().catch(() => ({}))
    throw new Error(d.message ?? res.statusText)
  }
}
