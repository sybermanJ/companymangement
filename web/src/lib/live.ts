// Live unread-notification count over server-sent events.
import { ref } from 'vue'

export const unread = ref(0)
let source: EventSource | null = null

export function startLive() {
  if (source || typeof EventSource === 'undefined') return
  source = new EventSource('/api/v1/notifications/stream', { withCredentials: true })
  source.addEventListener('unread', (e) => {
    unread.value = Number((e as MessageEvent).data) || 0
  })
  source.onerror = () => {
    // The browser reconnects by itself; after logout the stream ends with 401.
  }
}

export function stopLive() {
  source?.close()
  source = null
  unread.value = 0
}
