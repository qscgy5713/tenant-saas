/** 整頁導向外部網址(Stripe 結帳 / 客戶入口)。獨立成函式,測試才能攔截 */
export const redirectTo = (url: string) => window.location.assign(url)
