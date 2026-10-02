import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { ApiError } from '../../api/client'
import { PlusIcon, XIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import {
  MAX_RANGES_TOTAL,
  WEEKDAY_LABEL,
  WEEK_ORDER,
  countRanges,
  emptyWeek,
  fromHours,
  presetWeek,
  sameWeek,
  toHours,
  validateWeek,
  type WeekState,
} from '../../lib/hours'
import { getWorkingHours, putWorkingHours } from '../api'

/** 每週營業時間。時間都是店家當地時間,不隨夏令時間改變(例如「週一 09:00 開門」永遠是 09:00)。 */
export function WorkingHoursEditor({
  slug,
  userId,
  editable,
}: {
  slug: string
  userId: string
  editable: boolean
}) {
  const query = useQuery({
    queryKey: ['admin', 'hours', slug, userId],
    queryFn: () => getWorkingHours(slug, userId),
  })
  if (query.isPending) return <Loading label="載入營業時間…" />
  if (query.isError) return <ErrorState error={query.error} onRetry={() => query.refetch()} />
  // 資料載入後才掛上表單,表單的初始值就是伺服器上的內容
  return (
    <HoursForm
      key={JSON.stringify(query.data.hours)}
      slug={slug}
      userId={userId}
      initial={fromHours(query.data.hours)}
      editable={editable}
    />
  )
}

function HoursForm({
  slug,
  userId,
  initial,
  editable,
}: {
  slug: string
  userId: string
  initial: WeekState
  editable: boolean
}) {
  const queryClient = useQueryClient()
  const [week, setWeek] = useState<WeekState>(initial)
  const [saved, setSaved] = useState(false)
  const [showErrors, setShowErrors] = useState(false)

  const errors = validateWeek(week)
  const dirty = !sameWeek(week, initial)

  const mutation = useMutation({
    mutationFn: () => putWorkingHours(slug, userId, toHours(week)),
    onSuccess: (data) => {
      queryClient.setQueryData(['admin', 'hours', slug, userId], data)
      setSaved(true)
    },
  })

  const update = (day: number, ranges: WeekState[number]) => {
    setWeek((w) => ({ ...w, [day]: ranges }))
    setSaved(false)
  }

  const save = () => {
    setShowErrors(true)
    if (Object.keys(errors).length > 0) return
    mutation.mutate()
  }

  const apiError =
    mutation.error instanceof ApiError
      ? mutation.error.message
      : mutation.error
        ? '儲存失敗,請再試一次。'
        : null

  return (
    <div className="hours">
      {editable && (
        <div className="presets">
          <span className="muted small">快速設定:</span>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => {
              setWeek(presetWeek([1, 2, 3, 4, 5]))
              setSaved(false)
            }}
          >
            週一至週五 09:00–18:00
          </button>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => {
              setWeek(presetWeek([1, 2, 3, 4, 5, 6, 0]))
              setSaved(false)
            }}
          >
            每天 09:00–18:00
          </button>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => {
              setWeek(emptyWeek())
              setSaved(false)
            }}
          >
            全部公休
          </button>
        </div>
      )}

      <ul className="week">
        {WEEK_ORDER.map((day) => {
          const ranges = week[day]
          const error = showErrors ? errors[day] : undefined
          return (
            <li key={day} className={`week-day ${ranges.length ? '' : 'week-off'}`}>
              <div className="week-label">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={ranges.length > 0}
                    disabled={!editable}
                    aria-label={`${WEEKDAY_LABEL[day]}營業`}
                    onChange={(e) =>
                      update(day, e.target.checked ? [{ start: '09:00', end: '18:00' }] : [])
                    }
                  />
                  <span>{WEEKDAY_LABEL[day]}</span>
                </label>
              </div>
              <div className="week-ranges">
                {ranges.length === 0 && <span className="muted">公休</span>}
                {ranges.map((r, i) => (
                  <div key={i} className="range">
                    <input
                      type="time"
                      step={900}
                      value={r.start}
                      disabled={!editable}
                      aria-label={`${WEEKDAY_LABEL[day]}第 ${i + 1} 段開始時間`}
                      onChange={(e) =>
                        update(
                          day,
                          ranges.map((x, j) => (j === i ? { ...x, start: e.target.value } : x)),
                        )
                      }
                    />
                    <span aria-hidden="true">–</span>
                    <input
                      type="time"
                      step={900}
                      value={r.end}
                      disabled={!editable}
                      aria-label={`${WEEKDAY_LABEL[day]}第 ${i + 1} 段結束時間`}
                      onChange={(e) =>
                        update(
                          day,
                          ranges.map((x, j) => (j === i ? { ...x, end: e.target.value } : x)),
                        )
                      }
                    />
                    {editable && (
                      <button
                        type="button"
                        className="icon-btn icon-btn-sm"
                        aria-label={`移除${WEEKDAY_LABEL[day]}第 ${i + 1} 段`}
                        onClick={() =>
                          update(
                            day,
                            ranges.filter((_, j) => j !== i),
                          )
                        }
                      >
                        <XIcon width={14} height={14} />
                      </button>
                    )}
                  </div>
                ))}
                {editable && ranges.length > 0 && (
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    disabled={countRanges(week) >= MAX_RANGES_TOTAL}
                    onClick={() => update(day, [...ranges, { start: '13:00', end: '18:00' }])}
                  >
                    <PlusIcon width={14} height={14} />
                    加一段
                  </button>
                )}
                {error && <p className="field-msg">{error}</p>}
              </div>
            </li>
          )
        })}
      </ul>

      {apiError && <Notice tone="error">{apiError}</Notice>}
      {saved && <Notice tone="success">營業時間已儲存。</Notice>}

      {editable && (
        <div className="actions">
          <button
            type="button"
            className="btn btn-primary"
            disabled={!dirty || mutation.isPending}
            onClick={save}
          >
            {mutation.isPending ? '儲存中…' : '儲存營業時間'}
          </button>
          {dirty && (
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => {
                setWeek(initial)
                setShowErrors(false)
              }}
            >
              放棄變更
            </button>
          )}
          {dirty && <span className="muted small">有尚未儲存的變更</span>}
        </div>
      )}
    </div>
  )
}
