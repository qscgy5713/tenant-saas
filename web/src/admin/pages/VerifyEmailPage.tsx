import { verifyEmail } from '../api'
import { TokenLinkPage } from './TokenLinkPage'

/** 驗證信的連結:`/admin/verify#token=…` */
export function VerifyEmailPage() {
  return (
    <TokenLinkPage
      action={verifyEmail}
      pendingTitle="驗證中…"
      pendingText="正在確認你的 Email。"
      successTitle="Email 已驗證"
      failureTitle="驗證失敗"
      brokenText="這個驗證連結不完整。請回到信件重新點開,或登入後重新寄送。"
    />
  )
}
