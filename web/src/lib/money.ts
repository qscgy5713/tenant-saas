/** 後端只有 price_cents,沒有幣別。目前一律當新台幣的「分」顯示;0 顯示「免費」。 */
export function formatPrice(priceCents: number): string {
  if (priceCents === 0) return '免費'
  return new Intl.NumberFormat('zh-TW', {
    style: 'currency',
    currency: 'TWD',
    maximumFractionDigits: 0,
  }).format(priceCents / 100)
}

export function formatDuration(minutes: number): string {
  if (minutes < 60) return `${minutes} 分鐘`
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return m === 0 ? `${h} 小時` : `${h} 小時 ${m} 分鐘`
}
