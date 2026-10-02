import { useRef, useState, type FormEvent } from 'react'
import { type CustomerErrors, type CustomerForm, validateCustomer } from '../lib/validate'

export function DetailsForm({
  pending,
  onSubmit,
}: {
  pending: boolean
  onSubmit: (values: CustomerForm) => void
}) {
  const [values, setValues] = useState<CustomerForm>({ name: '', email: '', phone: '' })
  const [errors, setErrors] = useState<CustomerErrors>({})
  const formRef = useRef<HTMLFormElement>(null)

  const set = (key: keyof CustomerForm) => (e: React.ChangeEvent<HTMLInputElement>) => {
    setValues((v) => ({ ...v, [key]: e.target.value }))
    if (errors[key]) setErrors((prev) => ({ ...prev, [key]: undefined }))
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = validateCustomer(values)
    setErrors(found)
    const firstInvalid = (['name', 'email', 'phone'] as const).find((k) => found[k])
    if (firstInvalid) {
      // 把焦點移到第一個有問題的欄位,螢幕閱讀器與鍵盤使用者才知道錯在哪
      formRef.current?.querySelector<HTMLInputElement>(`[name="${firstInvalid}"]`)?.focus()
      return
    }
    onSubmit(values)
  }

  return (
    <form ref={formRef} className="form" onSubmit={submit} noValidate>
      <Field name="name" label="姓名" error={errors.name} required>
        <input
          name="name"
          type="text"
          autoComplete="name"
          value={values.name}
          onChange={set('name')}
          aria-invalid={!!errors.name}
          aria-describedby={errors.name ? 'err-name' : undefined}
          maxLength={200}
        />
      </Field>
      <Field name="email" label="Email" error={errors.email} required hint="預約確認信會寄到這裡">
        <input
          name="email"
          type="email"
          inputMode="email"
          autoComplete="email"
          value={values.email}
          onChange={set('email')}
          aria-invalid={!!errors.email}
          aria-describedby={errors.email ? 'err-email' : undefined}
        />
      </Field>
      <Field name="phone" label="手機(選填)" error={errors.phone}>
        <input
          name="phone"
          type="tel"
          inputMode="tel"
          autoComplete="tel"
          value={values.phone}
          onChange={set('phone')}
          aria-invalid={!!errors.phone}
          aria-describedby={errors.phone ? 'err-phone' : undefined}
        />
      </Field>
      <button type="submit" className="btn btn-primary btn-block" disabled={pending}>
        {pending ? '送出中…' : '送出預約申請'}
      </button>
      <p className="muted small">
        送出後我們會寄一封確認信給你,<strong>按下信中的連結確認後,預約才會成立</strong>。
      </p>
    </form>
  )
}

function Field({
  name,
  label,
  error,
  hint,
  required,
  children,
}: {
  name: keyof CustomerForm
  label: string
  error?: string
  hint?: string
  required?: boolean
  children: React.ReactElement<{ id?: string }>
}) {
  // 用 label 包住 input,點標籤就聚焦;錯誤訊息的 id 與 aria-describedby 對應
  return (
    <div className={`field ${error ? 'field-error' : ''}`}>
      <label>
        <span className="field-label">
          {label}
          {required && <span aria-hidden="true"> *</span>}
        </span>
        {children}
      </label>
      {hint && !error && <p className="field-hint">{hint}</p>}
      {error && (
        <p className="field-msg" id={`err-${name}`}>
          {error}
        </p>
      )}
    </div>
  )
}
