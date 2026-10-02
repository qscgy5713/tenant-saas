import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { ApiError } from '../api/client'
import {
  cancelBooking,
  confirmBooking,
  getBookingAvailability,
  rescheduleBooking,
} from '../api/public'
import { type SlotSource, useBooking } from '../api/queries'
import type { BookingStatus, PublicBooking } from '../api/types'
import { Shell } from '../components/Shell'
import { SlotPicker } from '../components/SlotPicker'
import { ErrorState, Loading, Notice } from '../components/States'
import type { TimeOption } from '../lib/slots'
import { formatDateTime, formatTime, tzLabel } from '../lib/time'
import { useNow } from '../lib/useNow'

const STATUS_LABEL: Record<BookingStatus, string> = {
  pending: '待確認',
  confirmed: '已確認',
  cancelled: '已取消',
  completed: '已完成',
  no_show: '未到',
}

type Mode = 'view' | 'reschedule' | 'cancel'

/** 信中連結的目的地:`/bookings/{token}`。確認要按按鈕才送出(不能自動送出:信件掃描器會先點開連結) */
export function ManagePage() {
  const { token = '' } = useParams()
  const queryClient = useQueryClient()
  const booking = useBooking(token)
  const [mode, setMode] = useState<Mode>('view')
  const [flash, setFlash] = useState<string | null>(null)
  const [pick, setPick] = useState<TimeOption | null>(null)
  const now = useNow()
  // 改期的時段來源:同一位員工與服務,並排除這筆預約自己(自己原本的時段不是忙碌)
  const customerRescheduleSource: SlotSource = {
    key: ['availability', 'booking', token],
    fetch: (from, to) => getBookingAvailability(token, from, to),
  }

  const done = (next: PublicBooking, message: string) => {
    queryClient.setQueryData(['booking', token], next)
    queryClient.invalidateQueries({ queryKey: ['availability'] })
    setMode('view')
    setPick(null)
    setFlash(message)
  }

  const confirm = useMutation({
    mutationFn: () => confirmBooking(token),
    onSuccess: (b) => done(b, '預約已確認!我們另外寄了一封確認信給你。'),
  })
  const cancel = useMutation({
    mutationFn: () => cancelBooking(token),
    onSuccess: (b) => done(b, '預約已取消。'),
  })
  const reschedule = useMutation({
    mutationFn: (start: string) => rescheduleBooking(token, start),
    onSuccess: (b) => done(b, '已改期。'),
  })

  if (booking.isPending)
    return (
      <Shell title="我的預約">
        <Loading />
      </Shell>
    )
  if (booking.isError) {
    const gone = booking.error instanceof ApiError && booking.error.status === 404
    return (
      <Shell title="我的預約">
        <ErrorState
          error={booking.error}
          title={gone ? '找不到這筆預約' : '發生問題'}
          onRetry={() => booking.refetch()}
        >
          {gone && <p className="muted">連結可能不完整或已失效。請使用確認信中的完整連結。</p>}
        </ErrorState>
      </Shell>
    )
  }

  const b = booking.data
  const tz = b.timezone
  const started = new Date(b.starts_at).getTime() <= now
  const active = b.status === 'pending' || b.status === 'confirmed'
  const changeable = active && !started

  const actionError = [confirm, cancel, reschedule].find((m) => m.isError)?.error
  const start = formatDateTime(b.starts_at, tz)

  return (
    <Shell title="我的預約" shopName={b.shop_name} shopSlug={b.shop_slug}>
      <div className="narrow">
        <h1 className="page-title">我的預約</h1>

        {flash && <Notice tone="success">{flash}</Notice>}
        {actionError && (
          <Notice tone="error">
            {actionError instanceof ApiError ? actionError.message : '操作失敗,請再試一次。'}
          </Notice>
        )}

        <article className="card booking-card">
          <div className="booking-head">
            <h2>{b.service_name}</h2>
            <span className={`badge badge-${b.status}`}>{STATUS_LABEL[b.status]}</span>
          </div>
          <dl className="facts">
            <div>
              <dt>店家</dt>
              <dd>{b.shop_name}</dd>
            </div>
            <div>
              <dt>時間</dt>
              <dd>
                <span>
                  {start} – {formatTime(b.ends_at, tz)}
                </span>
                <span className="muted small">{tzLabel(tz, b.starts_at)},店家當地時間</span>
              </dd>
            </div>
            <div>
              <dt>服務人員</dt>
              <dd>{b.staff_name}</dd>
            </div>
            <div>
              <dt>姓名</dt>
              <dd>{b.customer_name}</dd>
            </div>
          </dl>

          {b.status === 'pending' && !started && mode === 'view' && (
            <div className="confirm-box">
              <Notice tone="warning">
                <strong>這筆預約還沒成立。</strong>按下「確認預約」後,時段才會為你保留。
              </Notice>
              <button
                type="button"
                className="btn btn-primary btn-block"
                onClick={() => confirm.mutate()}
                disabled={confirm.isPending}
              >
                {confirm.isPending ? '確認中…' : '確認預約'}
              </button>
            </div>
          )}

          {b.status === 'cancelled' && <p className="muted">這筆預約已取消。</p>}
          {(b.status === 'completed' || b.status === 'no_show') && (
            <p className="muted">這筆預約已經結束。</p>
          )}
          {active && started && <p className="muted">預約時間已過,無法再變更。</p>}
        </article>

        {changeable && mode === 'view' && (
          <div className="actions">
            {b.status === 'confirmed' && (
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setMode('reschedule')}
              >
                改期
              </button>
            )}
            <button
              type="button"
              className="btn btn-danger-outline"
              onClick={() => setMode('cancel')}
            >
              {b.status === 'pending' ? '取消這筆申請' : '取消預約'}
            </button>
          </div>
        )}

        {mode === 'cancel' && changeable && (
          <section className="card panel" aria-label="取消確認">
            <h2>確定要取消嗎?</h2>
            <p className="muted">取消後時段會釋出給其他人,無法復原,需要的話得重新預約。</p>
            <div className="actions">
              <button
                type="button"
                className="btn btn-danger"
                onClick={() => cancel.mutate()}
                disabled={cancel.isPending}
              >
                {cancel.isPending ? '取消中…' : '確定取消'}
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => setMode('view')}>
                先不要
              </button>
            </div>
          </section>
        )}

        {mode === 'reschedule' && b.status === 'confirmed' && !started && (
          <section className="card panel" aria-label="改期">
            <h2>選擇新的時間</h2>
            <p className="muted small">
              服務人員維持不變:{b.staff_name}。目前的時段在改期完成前不會釋出。
            </p>
            <SlotPicker
              source={customerRescheduleSource}
              timezone={tz}
              selectedStart={pick?.start ?? null}
              onSelect={setPick}
            />
            <div className="actions">
              <button
                type="button"
                className="btn btn-primary"
                disabled={!pick || reschedule.isPending}
                onClick={() => pick && reschedule.mutate(pick.start)}
              >
                {reschedule.isPending
                  ? '改期中…'
                  : pick
                    ? `改到 ${formatDateTime(pick.start, tz)}`
                    : '請先選擇新時間'}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => {
                  setMode('view')
                  setPick(null)
                }}
              >
                取消改期
              </button>
            </div>
          </section>
        )}

        <p className="small muted footnote">
          請勿把這個頁面的網址轉給其他人:任何拿到連結的人都能取消或改期這筆預約。
        </p>
        <Link to={`/s/${b.shop_slug}`} className="link-btn">
          回到 {b.shop_name}
        </Link>
      </div>
    </Shell>
  )
}
