import { act, render, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { freezeNow } from '../../test/freeze'
import { MonthCalendar } from './MonthCalendar'
import { WeekCalendar } from './WeekCalendar'

beforeEach(() => freezeNow()) // 台北 2026-10-05(週一)10:00

// 台北 10/6 00:30,再讓視窗取得焦點(休眠醒來 / 切回分頁)
const pastMidnight = () =>
  act(() => {
    vi.setSystemTime(new Date('2026-10-05T16:30:00Z'))
    window.dispatchEvent(new Event('focus'))
  })

// 單獨渲染:不靠父層重新渲染,元件自己就要跟著換日。
// (在整頁裡父層也會換日、整棵樹重新渲染,元件即使每次重算也會碰巧正確,測不出差別。)
describe('日曆元件單獨使用時,「今天」的標示跟著換日', () => {
  it('週日曆', async () => {
    const { container } = render(
      <WeekCalendar date="2026-10-07" tz="Asia/Taipei" bookings={[]} onPick={() => {}} />,
    )
    const todayHeads = () => [...container.querySelectorAll('.cal-head-today')]
    expect(todayHeads()).toHaveLength(1)
    expect(todayHeads()[0]).toHaveTextContent('5')
    expect(todayHeads()[0]).toHaveTextContent('(今天)')

    pastMidnight()
    await waitFor(() => expect(todayHeads()[0]).toHaveTextContent('6'))
    expect(todayHeads()).toHaveLength(1)
    expect(container.querySelectorAll('.cal-col-today')).toHaveLength(1)
  })

  it('月曆', async () => {
    const { container } = render(
      <MonthCalendar
        date="2026-10-07"
        tz="Asia/Taipei"
        bookings={[]}
        onPick={() => {}}
        onOpenDay={() => {}}
      />,
    )
    const marked = () => container.querySelector('[class*="today"]')
    expect(marked()).not.toBeNull()
    expect(marked()).toHaveTextContent('5')
    pastMidnight()
    await waitFor(() => expect(marked()).toHaveTextContent('6'))
  })
})
