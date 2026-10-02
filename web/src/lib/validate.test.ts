import { describe, expect, it } from 'vitest'
import { validateCustomer, validateEmail } from './validate'

describe('validateEmail(規則與後端 normalize_email 一致)', () => {
  it.each(['a@example.com', ' a@example.com ', 'a.b+c@sub.example.co'])('合法:%s', (e) => {
    expect(validateEmail(e)).toBeNull()
  })

  it.each([
    '',
    '   ',
    'plain',
    'a@b',
    'a@@example.com',
    '@example.com',
    'a@.com',
    'a@example.',
    'a b@example.com',
  ])('不合法:%j', (e) => {
    expect(validateEmail(e)).not.toBeNull()
  })

  it('超過 254 字元', () => {
    expect(validateEmail(`${'a'.repeat(250)}@e.com`)).not.toBeNull()
  })
})

describe('validateCustomer', () => {
  const ok = { name: '王小明', email: 'a@example.com', phone: '' }

  it('必填欄位', () => {
    expect(validateCustomer(ok)).toEqual({})
    expect(validateCustomer({ ...ok, name: '  ' }).name).toBeDefined()
    expect(validateCustomer({ ...ok, email: '' }).email).toBeDefined()
  })

  it('長度以「字」計(中文一字算一個,不是位元組)', () => {
    expect(validateCustomer({ ...ok, name: '王'.repeat(100) }).name).toBeUndefined()
    expect(validateCustomer({ ...ok, name: '王'.repeat(101) }).name).toBeDefined()
    expect(validateCustomer({ ...ok, phone: '1'.repeat(30) }).phone).toBeUndefined()
    expect(validateCustomer({ ...ok, phone: '1'.repeat(31) }).phone).toBeDefined()
  })

  it('電話可以空白', () => {
    expect(validateCustomer({ ...ok, phone: '' }).phone).toBeUndefined()
  })
})
