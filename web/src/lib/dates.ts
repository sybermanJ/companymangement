// Calendar dates as 'YYYY-MM-DD' strings, computed in UTC so time zones never shift a day.

const DAY = 86_400_000

export function toDay(s: string): number {
  return Math.floor(Date.parse(`${s}T00:00:00Z`) / DAY)
}
export function fromDay(d: number): string {
  return new Date(d * DAY).toISOString().slice(0, 10)
}
export function todayDay(): number {
  const now = new Date()
  return Math.floor(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate()) / DAY)
}
/** Day of week, Monday = 0. */
export function weekday(d: number): number {
  return (new Date(d * DAY).getUTCDay() + 6) % 7
}
export function hours(minutes: number): string {
  const h = minutes / 60
  return Number.isInteger(h) ? String(h) : h.toFixed(1)
}
