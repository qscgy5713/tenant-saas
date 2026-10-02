import { useMutation, useQuery } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { ApiError } from '../../api/client'
import { getServices, getStaff } from '../../api/public'
import { SlotPicker } from '../../components/SlotPicker'
import { Notice } from '../../components/States'
import { formatDateTime } from '../../lib/time'
import type { TimeOption } from '../../lib/slots'
import { formatDuration, formatPrice } from '../../lib/money'
import { validateCustomer, type CustomerErrors, type CustomerForm } from '../../lib/validate'
import { createBooking } from '../api'
import { useShop } from '../ShopContext'
import { Modal } from './Modal'

/** 代客預約(電話、現場)。直接成立,不需要顧客再按確認連結;系統仍會寄確認信給顧客。 */
export function NewBookingModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean
  onClose: () => void
  onCreated: () => void
}) {
  return (
    <Modal open={open} title="新增預約" onClose={onClose} size="lg">
      <NewBookingForm onClose={onClose} onCreated={onCreated} />
    </Modal>
  )
}

function NewBookingForm({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { slug, shop, user, role } = useShop()
  const isStaff = role === 'staff'

  const [serviceId, setServiceId] = useState('')
  // null = 不指定;員工只能替自己建立預約
  const [staffId, setStaffId] = useState<string | null>(isStaff ? user.id : null)
  const [choice, setChoice] = useState<TimeOption | null>(null)
  const [customer, setCustomer] = useState<CustomerForm>({ name: '', email: '', phone: '' })
  const [errors, setErrors] = useState<CustomerErrors>({})

  const services = useQuery({
    queryKey: ['services-public', slug],
    queryFn: () => getServices(slug),
  })
  const staff = useQuery({
    queryKey: ['staff-public', slug, serviceId],
    queryFn: () => getStaff(slug, serviceId),
    enabled: !!serviceId,
  })

  const service = services.data?.find((s) => s.id === serviceId)

  const mutation = useMutation({
    mutationFn: () =>
      createBooking(slug, {
        service_id: serviceId,
        staff_id: staffId ?? undefined,
        start: choice!.start,
        customer: {
          name: customer.name.trim(),
          email: customer.email.trim(),
          phone: customer.phone.trim() || undefined,
        },
      }),
    onSuccess: onCreated,
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!choice) return
    const found = validateCustomer(customer)
    setErrors(found)
    if (Object.keys(found).length === 0) mutation.mutate()
  }

  const error = mutation.error
  const set = (key: keyof CustomerForm) => (e: React.ChangeEvent<HTMLInputElement>) => {
    setCustomer((c) => ({ ...c, [key]: e.target.value }))
    if (errors[key]) setErrors((prev) => ({ ...prev, [key]: undefined }))
  }

  return (
    <form className="new-booking" onSubmit={submit} noValidate>
      <div className="field">
        <label>
          <span className="field-label">服務</span>
          <select
            value={serviceId}
            onChange={(e) => {
              setServiceId(e.target.value)
              setChoice(null)
              if (!isStaff) setStaffId(null)
            }}
          >
            <option value="">請選擇服務…</option>
            {services.data?.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}({formatDuration(s.duration_minutes)} · {formatPrice(s.price_cents)})
              </option>
            ))}
          </select>
        </label>
      </div>

      {serviceId && !isStaff && (staff.data?.length ?? 0) > 1 && (
        <div className="field">
          <label>
            <span className="field-label">服務人員</span>
            <select
              value={staffId ?? ''}
              onChange={(e) => {
                setStaffId(e.target.value || null)
                setChoice(null)
              }}
            >
              <option value="">不指定(由系統安排)</option>
              {staff.data?.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      {serviceId && (
        <SlotPicker
          key={`${serviceId}-${staffId ?? 'any'}`}
          slug={slug}
          serviceId={serviceId}
          timezone={shop.timezone}
          staffId={staffId}
          selectedStart={choice?.start ?? null}
          onSelect={setChoice}
        />
      )}

      {choice && service && (
        <>
          <Notice tone="info">
            <strong>{service.name}</strong> · {formatDateTime(choice.start, shop.timezone)}
          </Notice>
          <div className="field-row">
            <div className={`field ${errors.name ? 'field-error' : ''}`}>
              <label>
                <span className="field-label">顧客姓名</span>
                <input
                  name="name"
                  value={customer.name}
                  onChange={set('name')}
                  aria-invalid={!!errors.name}
                />
              </label>
              {errors.name && <p className="field-msg">{errors.name}</p>}
            </div>
            <div className={`field ${errors.phone ? 'field-error' : ''}`}>
              <label>
                <span className="field-label">手機(選填)</span>
                <input
                  name="phone"
                  type="tel"
                  value={customer.phone}
                  onChange={set('phone')}
                  aria-invalid={!!errors.phone}
                />
              </label>
              {errors.phone && <p className="field-msg">{errors.phone}</p>}
            </div>
          </div>
          <div className={`field ${errors.email ? 'field-error' : ''}`}>
            <label>
              <span className="field-label">顧客 Email</span>
              <input
                name="email"
                type="email"
                value={customer.email}
                onChange={set('email')}
                aria-invalid={!!errors.email}
              />
            </label>
            {errors.email ? (
              <p className="field-msg">{errors.email}</p>
            ) : (
              <p className="field-hint">確認信會寄到這裡,顧客可以用信中的連結改期或取消。</p>
            )}
          </div>
        </>
      )}

      {error && (
        <Notice tone="error">
          {error instanceof ApiError ? error.message : '建立失敗,請再試一次。'}
        </Notice>
      )}

      <div className="actions">
        <button type="submit" className="btn btn-primary" disabled={!choice || mutation.isPending}>
          {mutation.isPending ? '建立中…' : '建立預約'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </form>
  )
}
