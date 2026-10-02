import { describe, expect, it } from 'vitest'
import { meterStep } from './meter'

describe('meterStep', () => {
  it('沒有用量 → 0', () => {
    expect(meterStep(0, 50)).toBe(0)
    expect(meterStep(-3, 50)).toBe(0)
  })

  it('有用量但很小 → 至少一格,不會看起來是空的(10 / 1000 = 1%)', () => {
    expect(meterStep(10, 1000)).toBe(5)
    expect(meterStep(1, 100000)).toBe(5)
  })

  it('一般比例四捨五入到 5%', () => {
    expect(meterStep(3, 10)).toBe(30)
    expect(meterStep(3, 50)).toBe(5)
    expect(meterStep(1, 3)).toBe(35)
  })

  it('幾乎滿但還沒滿 → 最多 95,不能顯示成滿格(會誤導成已額滿)', () => {
    expect(meterStep(999, 1000)).toBe(95)
    expect(meterStep(49, 50)).toBe(95)
  })

  it('剛好滿 / 超過(降級後既有資料超出新上限)→ 100', () => {
    expect(meterStep(2, 2)).toBe(100)
    expect(meterStep(9, 5)).toBe(100)
  })

  it('上限為 0 或不合理 → 0,不會除以零', () => {
    expect(meterStep(5, 0)).toBe(0)
    expect(Number.isFinite(meterStep(5, 0))).toBe(true)
  })

  it('輸出一定是 CSS 有定義的格(0–100 的 5 的倍數)', () => {
    for (let used = 0; used <= 120; used += 7) {
      for (const max of [1, 2, 5, 50, 1000]) {
        const step = meterStep(used, max)
        expect(step % 5).toBe(0)
        expect(step).toBeGreaterThanOrEqual(0)
        expect(step).toBeLessThanOrEqual(100)
      }
    }
  })
})
