import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { ApiError } from '../../api/client'
import { PlusIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import { formatDuration, formatPrice } from '../../lib/money'
import { validateName } from '../../lib/validate'
import {
  createService,
  deleteService,
  getPlan,
  listServices,
  updateService,
  type ServiceInput,
} from '../api'
import { Modal, ConfirmDialog } from '../components/Modal'
import { useShop } from '../ShopContext'
import type { AdminService } from '../types'

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? '操作失敗,請再試一次。' : null

export function ServicesPage() {
  const { slug, canManage } = useShop()
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<AdminService | 'new' | null>(null)
  const [deleting, setDeleting] = useState<AdminService | null>(null)

  const services = useQuery({
    queryKey: ['admin', 'services', slug],
    queryFn: () => listServices(slug),
  })
  const plan = useQuery({
    queryKey: ['admin', 'plan', slug],
    queryFn: () => getPlan(slug),
    enabled: canManage,
    staleTime: 30_000,
  })

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ['admin', 'services', slug] })
    queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
    queryClient.invalidateQueries({ queryKey: ['services-public', slug] })
  }

  const toggle = useMutation({
    mutationFn: (s: AdminService) => updateService(slug, s.id, { active: !s.active }),
    onSuccess: refresh,
  })
  const remove = useMutation({
    mutationFn: (s: AdminService) => deleteService(slug, s.id),
    onSuccess: () => {
      refresh()
      setDeleting(null)
    },
  })

  const items = services.data?.items ?? []
  const active = items.filter((s) => s.active).length
  const limit = plan.data?.plan.max_services

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">服務</h1>
          <p className="muted small">
            顧客在預約頁看到的服務項目。
            {canManage ? `啟用中 ${active}${limit ? ` / 上限 ${limit}` : ''}` : ''}
          </p>
        </div>
        {canManage && (
          <div className="page-actions">
            <button type="button" className="btn btn-primary" onClick={() => setEditing('new')}>
              <PlusIcon width={16} height={16} />
              新增服務
            </button>
          </div>
        )}
      </div>

      {!canManage && <Notice tone="info">只有擁有者與管理者可以新增或修改服務。</Notice>}
      {toggle.error && <Notice tone="error">{errorText(toggle.error)}</Notice>}

      {services.isPending && <Loading label="載入服務…" />}
      {services.isError && <ErrorState error={services.error} onRetry={() => services.refetch()} />}

      {services.data && items.length === 0 && (
        <div className="empty-card">
          <h2>還沒有服務</h2>
          <p className="muted">新增第一項服務(例如剪髮),顧客才能在預約頁選擇。</p>
          {canManage && (
            <button type="button" className="btn btn-primary" onClick={() => setEditing('new')}>
              新增服務
            </button>
          )}
        </div>
      )}

      <ul className="rows">
        {items.map((s) => (
          <li key={s.id} className={`row service-row ${s.active ? '' : 'row-off'}`}>
            <div className="row-main">
              <div className="row-title">
                {s.name}
                {!s.active && <span className="chip chip-muted badge-inline">已停用</span>}
              </div>
              <div className="row-sub">
                {formatDuration(s.duration_minutes)} · {formatPrice(s.price_cents)}
              </div>
            </div>
            {canManage && (
              <div className="row-actions">
                <button
                  type="button"
                  className="switch"
                  role="switch"
                  aria-checked={s.active}
                  aria-label={`${s.name}:${s.active ? '啟用中' : '已停用'}`}
                  disabled={toggle.isPending}
                  onClick={() => toggle.mutate(s)}
                >
                  <span className="switch-knob" />
                </button>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => setEditing(s)}
                >
                  編輯
                </button>
                <button
                  type="button"
                  className="btn btn-danger-outline btn-sm"
                  onClick={() => setDeleting(s)}
                >
                  刪除
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>

      <ServiceModal
        open={editing !== null}
        service={editing === 'new' ? null : editing}
        onClose={() => setEditing(null)}
        onSaved={() => {
          setEditing(null)
          refresh()
        }}
      />
      <ConfirmDialog
        open={deleting !== null}
        title="刪除這項服務?"
        message={
          <>
            「{deleting?.name}
            」會被永久刪除。如果它已經有預約紀錄就無法刪除,請改為「停用」,停用後顧客看不到,歷史預約仍保留。
          </>
        }
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

function ServiceModal({
  open,
  service,
  onClose,
  onSaved,
}: {
  open: boolean
  service: AdminService | null
  onClose: () => void
  onSaved: () => void
}) {
  return (
    <Modal open={open} title={service ? '編輯服務' : '新增服務'} onClose={onClose}>
      <ServiceForm service={service} onClose={onClose} onSaved={onSaved} />
    </Modal>
  )
}

function ServiceForm({
  service,
  onClose,
  onSaved,
}: {
  service: AdminService | null
  onClose: () => void
  onSaved: () => void
}) {
  const { slug } = useShop()
  const [name, setName] = useState(service?.name ?? '')
  const [minutes, setMinutes] = useState(String(service?.duration_minutes ?? 60))
  const [price, setPrice] = useState(String((service?.price_cents ?? 0) / 100))
  const [errors, setErrors] = useState<{ name?: string; minutes?: string; price?: string }>({})

  const mutation = useMutation({
    mutationFn: (input: ServiceInput) =>
      service ? updateService(slug, service.id, input) : createService(slug, input),
    onSuccess: onSaved,
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const duration = Number(minutes)
    const yuan = Number(price)
    const found = {
      name: validateName(name, '服務名稱') ?? undefined,
      minutes:
        Number.isInteger(duration) && duration >= 5 && duration <= 1440
          ? undefined
          : '時長需為 5–1440 分鐘的整數',
      price:
        Number.isInteger(yuan) && yuan >= 0 && yuan <= 100_000
          ? undefined
          : '價格需為 0–100,000 的整數(元)',
    }
    setErrors(found)
    if (found.name || found.minutes || found.price) return
    mutation.mutate({ name: name.trim(), duration_minutes: duration, price_cents: yuan * 100 })
  }

  return (
    <form className="form-plain new-booking" onSubmit={submit} noValidate>
      {mutation.error && <Notice tone="error">{errorText(mutation.error)}</Notice>}
      <div className={`field ${errors.name ? 'field-error' : ''}`}>
        <label>
          <span className="field-label">服務名稱</span>
          <input
            name="name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-invalid={!!errors.name}
          />
        </label>
        {errors.name && <p className="field-msg">{errors.name}</p>}
      </div>
      <div className="field-row">
        <div className={`field ${errors.minutes ? 'field-error' : ''}`}>
          <label>
            <span className="field-label">時長(分鐘)</span>
            <input
              name="minutes"
              type="number"
              inputMode="numeric"
              min={5}
              max={1440}
              step={5}
              value={minutes}
              onChange={(e) => setMinutes(e.target.value)}
              aria-invalid={!!errors.minutes}
            />
          </label>
          {errors.minutes && <p className="field-msg">{errors.minutes}</p>}
        </div>
        <div className={`field ${errors.price ? 'field-error' : ''}`}>
          <label>
            <span className="field-label">價格(元)</span>
            <input
              name="price"
              type="number"
              inputMode="numeric"
              min={0}
              value={price}
              onChange={(e) => setPrice(e.target.value)}
              aria-invalid={!!errors.price}
            />
          </label>
          {errors.price ? (
            <p className="field-msg">{errors.price}</p>
          ) : (
            <p className="field-hint">0 代表免費</p>
          )}
        </div>
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
