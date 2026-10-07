/**
 * 電話 → `tel:` 連結(RFC 3966)。`#` 後面是分機:要放在 `;ext=`,不能直接接在號碼後面
 * (「02-1234-5678 #12」若變成 …567812,撥出去是另一個號碼)。其餘非數字(空白、連字號、括號)都拿掉,只留開頭的 +
 */
export function telHref(phone: string): string {
  const [main, ...extension] = phone.split('#')
  const number = main.replace(/[^0-9+]/g, '')
  const ext = extension.join('').replace(/\D/g, '')
  return `tel:${number}${ext ? `;ext=${ext}` : ''}`
}
