// Money is stored in minor units; the exponent depends on the currency.
const ZERO = new Set(['JPY', 'KRW', 'VND', 'CLP', 'ISK', 'UGX', 'XAF', 'XOF', 'PYG', 'RWF', 'KMF', 'GNF', 'DJF', 'BIF', 'VUV', 'XPF'])
const THREE = new Set(['KWD', 'BHD', 'OMR', 'JOD', 'TND', 'LYD', 'IQD'])

export function exponent(ccy: string): number {
  return ZERO.has(ccy) ? 0 : THREE.has(ccy) ? 3 : 2
}

export function toMinor(major: string | number, ccy: string): number {
  return Math.round(Number(major) * 10 ** exponent(ccy))
}

export function toMajor(minor: number, ccy: string): number {
  return minor / 10 ** exponent(ccy)
}

export function money(minor: number | null | undefined, ccy: string, locale = 'zh-CN'): string {
  if (minor === null || minor === undefined) return '—'
  const e = exponent(ccy)
  return `${ccy} ${new Intl.NumberFormat(locale, { minimumFractionDigits: e, maximumFractionDigits: e }).format(minor / 10 ** e)}`
}
