import { ApiError } from './api'

/** Map an API error to a translated, user-facing message. */
export function errorText(e: unknown, t: (k: string, v?: Record<string, unknown>) => string): string {
  if (e instanceof ApiError) {
    if (e.code === 'too_many_attempts') return t('auth.tooMany')
    if (e.code === 'bad_credentials') return t('auth.wrong')
    if (e.code === 'invalid_token') return t('auth.badLink')
    return t('common.error', { msg: e.message })
  }
  return t('common.error', { msg: String(e) })
}
