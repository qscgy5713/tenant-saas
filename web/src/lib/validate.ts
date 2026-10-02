// 與後端 src/validation.rs 與 booking.rs 的規則一致(後端仍是最終把關,這裡只是讓使用者即時知道哪裡要改)

export interface CustomerForm {
  name: string
  email: string
  phone: string
}

export type CustomerErrors = Partial<Record<keyof CustomerForm, string>>

export function validateEmail(raw: string): string | null {
  const email = raw.trim()
  if (!email) return '請輸入 Email'
  const at = email.indexOf('@')
  const domain = email.slice(at + 1)
  const ok =
    email.length <= 254 &&
    !/\s/.test(email) &&
    at > 0 &&
    email.indexOf('@') === email.lastIndexOf('@') &&
    domain.includes('.') &&
    !domain.startsWith('.') &&
    !domain.endsWith('.')
  return ok ? null : 'Email 格式不正確'
}

export function validateCustomer(form: CustomerForm): CustomerErrors {
  const errors: CustomerErrors = {}
  const name = form.name.trim()
  if (!name) errors.name = '請輸入姓名'
  else if ([...name].length > 100) errors.name = '姓名最多 100 字'

  const emailError = validateEmail(form.email)
  if (emailError) errors.email = emailError

  const phone = form.phone.trim()
  if ([...phone].length > 30) errors.phone = '電話最多 30 字'
  return errors
}

// 與後端 src/routes/tenants.rs 的 validate_slug 一致
const RESERVED_SLUGS = ['www', 'api', 'app', 'admin', 'static', 'assets', 'mail', 'public']

export function validateSlug(raw: string): string | null {
  const slug = raw.trim()
  if (!slug) return '請輸入預約頁網址代稱'
  if (slug.length < 3 || slug.length > 40) return '網址代稱需為 3–40 個字'
  if (!/^[a-z0-9-]+$/.test(slug)) return '只能使用小寫英文字母、數字與連字號(-)'
  if (slug.startsWith('-') || slug.endsWith('-')) return '不能以連字號開頭或結尾'
  if (RESERVED_SLUGS.includes(slug)) return '這個代稱是保留字,請換一個'
  return null
}

export function validatePassword(password: string): string | null {
  const length = [...password].length
  if (length < 8) return '密碼至少 8 個字'
  if (length > 128) return '密碼最多 128 個字'
  return null
}

export function validateName(raw: string, label = '名稱', max = 100): string | null {
  const name = raw.trim()
  if (!name) return `請輸入${label}`
  if ([...name].length > max) return `${label}最多 ${max} 字`
  return null
}
