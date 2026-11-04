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
}

export interface Membership { tenant_id: string; tenant_name: string; is_admin: boolean; is_finance: boolean }
export interface Me { user_id: string; tenant_id: string; email: string; name: string; email_verified: boolean; memberships: Membership[] }
export interface Department { id: string; parent_id: string | null; name: string; head_user_id: string | null }
export interface Member { user_id: string; email: string; display_name: string; status: string; is_admin: boolean; is_finance: boolean }
export interface Role { id: string; department_id: string | null; name: string }
export interface Assignment { user_id: string; role_id: string; role_name: string; department_id: string; department_name: string }
