import { confirmEmailChange } from '../api'
import { TokenLinkPage } from './TokenLinkPage'

/** 寄到新地址的確認連結:`/admin/change-email#token=…`。點了才真的更換 */
export function ChangeEmailPage() {
  return (
    <TokenLinkPage
      action={confirmEmailChange}
      pendingTitle="更換中…"
      pendingText="正在確認你的新 Email。"
      successTitle="Email 已更換"
      failureTitle="更換失敗"
      brokenText="這個連結不完整。請回到信件重新點開,或登入後重新申請。"
    />
  )
}
