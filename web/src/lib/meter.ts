/**
 * 用量條的寬度 class 名稱(w-0 … w-100,每 5% 一格)。
 * 有用量就至少顯示一格:10 / 1000 只有 1%,四捨五入成 0 會讓條看起來是空的,像是沒有用量。
 * 滿了才顯示 100:99.9% 不能顯示成滿格,會讓人以為已經額滿。
 */
export function meterStep(used: number, max: number): number {
  if (max <= 0 || used <= 0) return 0
  const ratio = Math.min(1, used / max)
  if (ratio >= 1) return 100
  return Math.min(95, Math.max(5, Math.round(ratio * 20) * 5))
}
