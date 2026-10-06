import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { ApiError } from '../../api/client'
import { ErrorState, Loading, Notice } from '../../components/States'
import { addDays, formatDateTime, startOfDay, todayYmd, zonedToUtc } from '../../lib/time'
import { createTimeOff, deleteTimeOff, listTimeOff } from '../api'
import { ConfirmDialog, Modal } from './Modal'
import type { TimeOff } from '../types'

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? '操作失敗,請再試一次。' : null

/** 休假 / 不可預約的時段。這段時間顧客預約頁不會出現這位成員的空檔。 */
export function TimeOffSection({
  slug,
  userId,
  timezone,
  editable,
}: {
  slug: string
  userId: string
  timezone: string
  editable: boolean
}) {
  const queryClient = useQueryClient()
  const [adding, setAdding] = useState(false)
  const [deleting, setDeleting] = useState<TimeOff | null>(null)
  const [showPast, setShowPast] = useState(false)
  const [now] = useState(() => Date.now())

  // 分頁:一次載入一頁,「載入更多」再抓下一頁。預設只看還沒結束的,已結束的另外看
  const query = useInfiniteQuery({
    queryKey: ['admin', 'time-off', slug, userId, showPast ? 'past' : 'upcoming'],
    queryFn: ({ pageParam }) => listTimeOff(slug, userId, { past: showPast, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last) => (last.has_more ? last.offset + last.limit : undefined),
  })
  const items = query.data?.pages.flatMap((p) => p.items)
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ['admin', 'time-off', slug, userId] })

  const remove = useMutation({
    mutationFn: (t: TimeOff) => deleteTimeOff(slug, t.id),
    onSuccess: () => {
      setDeleting(null)
      refresh()
    },
  })

  return (
    <div>
      {editable && (
        <div className="actions">
          <button type="button" className="btn btn-secondary" onClick={() => setAdding(true)}>
            新增休假
          </button>
        </div>
      )}
      {query.isPending && <Loading label="載入休假…" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => query.refetch()} />}
      <div className="actions">
        <label className="check">
          <input
            type="checkbox"
            checked={showPast}
            onChange={(e) => setShowPast(e.target.checked)}
          />
          <span>只看已結束的</span>
        </label>
      </div>
      {items?.length === 0 && (
        <p className="muted">{showPast ? '沒有已結束的休假。' : '沒有設定休假。'}</p>
      )}
      <ul className="rows">
        {items?.map((t) => {
          const past = new Date(t.ends_at).getTime() <= now
          return (
            <li key={t.id} className={`row ${past ? 'row-off' : ''}`}>
              <div className="row-main">
                <div className="row-title">
                  {formatDateTime(t.starts_at, timezone)} → {formatDateTime(t.ends_at, timezone)}
                  {past && <span className="chip chip-muted badge-inline">已結束</span>}
                </div>
                {t.reason && <div className="row-sub">{t.reason}</div>}
              </div>
              {editable && (
                <div className="row-actions">
                  <button
                    type="button"
                    className="btn btn-danger-outline btn-sm"
                    onClick={() => setDeleting(t)}
                  >
                    刪除
                  </button>
                </div>
              )}
            </li>
          )
        })}
      </ul>
      {query.hasNextPage && (
        <div className="actions">
          <button
            type="button"
            className="btn btn-secondary"
            disabled={query.isFetchingNextPage}
            onClick={() => query.fetchNextPage()}
          >
            {query.isFetchingNextPage ? '載入中…' : '載入更多'}
          </button>
        </div>
      )}
      {query.isFetchNextPageError && <Notice tone="error">載入更多失敗,請再試一次。</Notice>}
      <p className="muted small">休假原因只有管理者與本人看得到。稽核日誌不會記錄原因。</p>

      <Modal open={adding} title="新增休假" onClose={() => setAdding(false)}>
        <TimeOffForm
          slug={slug}
          userId={userId}
          timezone={timezone}
          onClose={() => setAdding(false)}
          onSaved={() => {
            setAdding(false)
            refresh()
          }}
        />
      </Modal>
      <ConfirmDialog
        open={deleting !== null}
        title="刪除這段休假?"
        message="刪除後,這段時間會重新開放給顧客預約。"
        confirmLabel="刪除"
        danger
        pending={remove.isPending}
        error={errorText(remove.error)}
        onConfirm={() => deleting && remove.mutate(deleting)}
        onClose={() => {
          setDeleting(null)
          remove.reset()
        }}
      />
    </div>
  )
}

function TimeOffForm({
  slug,
  userId,
  timezone,
  onClose,
  onSaved,
}: {
  slug: string
  userId: string
  timezone: string
  onClose: () => void
  onSaved: () => void
}) {
  const today = todayYmd(timezone)
  const [allDay, setAllDay] = useState(true)
  const [startDate, setStartDate] = useState(today)
  const [endDate, setEndDate] = useState(today)
  const [startTime, setStartTime] = useState('09:00')
  const [endTime, setEndTime] = useState('18:00')
  const [reason, setReason] = useState('')
  const [error, setError] = useState<string | null>(null)

  const mutation = useMutation({
    mutationFn: (range: { starts_at: string; ends_at: string }) =>
      createTimeOff(slug, userId, { ...range, reason: reason.trim() || undefined }),
    onSuccess: onSaved,
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    // 整天:從開始日的 00:00 到結束日「隔天」的 00:00(結束日當天也算休假)。時間都以店家時區解讀
    const start = allDay
      ? startOfDay(startDate, timezone)
      : zonedToUtc(startDate, startTime, timezone)
    const end = allDay
      ? startOfDay(addDays(endDate, 1), timezone)
      : zonedToUtc(endDate, endTime, timezone)
    if (!startDate || !endDate || Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
      setError('請選擇完整的日期與時間')
      return
    }
    if (end <= start) {
      setError('結束時間必須晚於開始時間')
      return
    }
    if ([...reason].length > 200) {
      setError('原因最多 200 字')
      return
    }
    setError(null)
    mutation.mutate({ starts_at: start.toISOString(), ends_at: end.toISOString() })
  }

  const shown = error ?? errorText(mutation.error)

  return (
    <form className="form-plain new-booking" onSubmit={submit} noValidate>
      {shown && <Notice tone="error">{shown}</Notice>}
      <label className="check">
        <input type="checkbox" checked={allDay} onChange={(e) => setAllDay(e.target.checked)} />
        <span>整天</span>
      </label>
      <div className="field-row">
        <div className="field">
          <label>
            <span className="field-label">開始日期</span>
            <input type="date" value={startDate} onChange={(e) => setStartDate(e.target.value)} />
          </label>
        </div>
        {!allDay && (
          <div className="field">
            <label>
              <span className="field-label">開始時間</span>
              <input
                type="time"
                step={900}
                value={startTime}
                onChange={(e) => setStartTime(e.target.value)}
              />
            </label>
          </div>
        )}
      </div>
      <div className="field-row">
        <div className="field">
          <label>
            <span className="field-label">結束日期{allDay ? '(含)' : ''}</span>
            <input type="date" value={endDate} onChange={(e) => setEndDate(e.target.value)} />
          </label>
        </div>
        {!allDay && (
          <div className="field">
            <label>
              <span className="field-label">結束時間</span>
              <input
                type="time"
                step={900}
                value={endTime}
                onChange={(e) => setEndTime(e.target.value)}
              />
            </label>
          </div>
        )}
      </div>
      <div className="field">
        <label>
          <span className="field-label">原因(選填)</span>
          <input value={reason} onChange={(e) => setReason(e.target.value)} maxLength={200} />
        </label>
        <p className="field-hint">時間以店家當地時間({timezone})解讀。</p>
      </div>
      <div className="actions">
        <button type="submit" className="btn btn-primary" disabled={mutation.isPending}>
          {mutation.isPending ? '儲存中…' : '儲存'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </form>
  )
}
