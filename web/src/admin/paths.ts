/** 登入後只回到站內的後台 / 邀請頁,避免被塞入外部網址的開放式重新導向 */
export function safeReturnPath(value: unknown): string {
  if (typeof value !== 'string') return '/admin'
  // 正規式要求開頭是 `/admin` 或 `/invitations`,所以 `//evil.com`、`https://…`、`/\\evil.com` 都不會匹配
  const ok = /^\/(admin|invitations)(\/|$|\?|#)/.test(value)
  return ok ? value : '/admin'
}
