import { describe, expect, it } from 'vitest'
import { formatDuration, formatPrice } from './money'

describe('formatPrice', () => {
  it('0 是免費', () => expect(formatPrice(0)).toBe('免費'))
  it('分轉元,千分位', () => {
    expect(formatPrice(50_000)).toMatch(/500/)
    expect(formatPrice(123_456_00)).toMatch(/123,456/)
  })
})

describe('formatDuration', () => {
  it('分鐘 / 小時 / 混合', () => {
    expect(formatDuration(30)).toBe('30 分鐘')
    expect(formatDuration(60)).toBe('1 小時')
    expect(formatDuration(90)).toBe('1 小時 30 分鐘')
  })
})
