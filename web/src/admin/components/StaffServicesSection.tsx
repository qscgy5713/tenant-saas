import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { ApiError } from '../../api/client'
import { ErrorState, Loading, Notice } from '../../components/States'
import { formatDuration, formatPrice } from '../../lib/money'
import { getStaffServices, listServices, putStaffServices } from '../api'

/** 這位成員可以提供哪些服務。沒勾任何服務 = 顧客在預約頁選不到他 */
export function StaffServicesSection({
  slug,
  userId,
  editable,
}: {
  slug: string
  userId: string
  editable: boolean
}) {
  const services = useQuery({
    queryKey: ['admin', 'services', slug],
    queryFn: () => listServices(slug),
  })
  const assigned = useQuery({
    queryKey: ['admin', 'staff-services', slug, userId],
    queryFn: () => getStaffServices(slug, userId),
  })

  if (services.isPending || assigned.isPending) return <Loading label="載入服務…" />
  if (services.isError)
    return <ErrorState error={services.error} onRetry={() => services.refetch()} />
  if (assigned.isError)
    return <ErrorState error={assigned.error} onRetry={() => assigned.refetch()} />

  return (
    <ServicesForm
      key={assigned.data.service_ids.join(',')}
      slug={slug}
      userId={userId}
      editable={editable}
      services={services.data.items}
      initial={assigned.data.service_ids}
    />
  )
}

function ServicesForm({
  slug,
  userId,
  editable,
  services,
  initial,
}: {
  slug: string
  userId: string
  editable: boolean
  services: {
    id: string
    name: string
    duration_minutes: number
    price_cents: number
    active: boolean
  }[]
  initial: string[]
}) {
  const queryClient = useQueryClient()
  const [selected, setSelected] = useState(new Set(initial))
  const [saved, setSaved] = useState(false)

  const dirty = selected.size !== initial.length || initial.some((id) => !selected.has(id))

  const mutation = useMutation({
    mutationFn: () => putStaffServices(slug, userId, [...selected]),
    onSuccess: (data) => {
      queryClient.setQueryData(['admin', 'staff-services', slug, userId], data)
      queryClient.invalidateQueries({ queryKey: ['staff-public', slug] })
      setSaved(true)
    },
  })

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
    setSaved(false)
  }

  if (services.length === 0) return <p className="muted">還沒有任何服務,請先到「服務」頁新增。</p>

  return (
    <div>
      <ul className="checklist">
        {services.map((s) => (
          <li key={s.id}>
            <label className="check">
              <input
                type="checkbox"
                checked={selected.has(s.id)}
                disabled={!editable}
                onChange={() => toggle(s.id)}
              />
              <span>
                <strong>{s.name}</strong>
                {!s.active && <span className="chip chip-muted badge-inline">已停用</span>}
                <span className="muted small">
                  {' '}
                  {formatDuration(s.duration_minutes)} · {formatPrice(s.price_cents)}
                </span>
              </span>
            </label>
          </li>
        ))}
      </ul>
      {selected.size === 0 && (
        <Notice tone="warning">沒有勾選任何服務時,顧客在預約頁選不到這位成員。</Notice>
      )}
      {mutation.error && (
        <Notice tone="error">
          {mutation.error instanceof ApiError ? mutation.error.message : '儲存失敗,請再試一次。'}
        </Notice>
      )}
      {saved && <Notice tone="success">已儲存。</Notice>}
      {editable && (
        <div className="actions">
          <button
            type="button"
            className="btn btn-primary"
            disabled={!dirty || mutation.isPending}
            onClick={() => mutation.mutate()}
          >
            {mutation.isPending ? '儲存中…' : '儲存'}
          </button>
        </div>
      )}
    </div>
  )
}
