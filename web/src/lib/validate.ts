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
