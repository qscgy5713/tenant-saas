import './admin.css'
import { AcceptInvitationPage } from './pages/AcceptInvitationPage'
import { useClearCacheOnLogout } from './useClearCacheOnLogout'

/** 邀請信的連結入口 `/invitations/accept`(同樣用 React.lazy 載入) */
export default function InvitationEntry() {
  useClearCacheOnLogout()
  return <AcceptInvitationPage />
}
