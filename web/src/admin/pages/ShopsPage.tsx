import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Avatar } from '../../components/Avatar'
import { ArrowRightIcon, LogOutIcon, PlusIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import { useTitle } from '../../lib/useTitle'
import { validateEmail, validateName, validateSlug } from '../../lib/validate'
import {
  createShop,
  listAccountEvents,
  listShops,
  requestEmailChange,
  resendVerification,
} from '../api'
import { ConfirmDialog, Modal } from '../components/Modal'
import { TextField } from '../components/AuthCard'
import { useDeleteAccount, useLogout, useLogoutAll } from '../logout'
import { TIMEZONES } from '../timezones'
import { ROLE_LABEL } from '../ShopContext'
import { restoreSession, useSession } from '../session'
import { formatDateTime, tzLabel } from '../../lib/time'
import type { AccountEvent } from '../types'

export function ShopsPage() {
  useTitle('選擇店家 · 店家後台')
  const session = useSession()
  const logout = useLogout()
  const shops = useQuery({ queryKey: ['admin', 'shops'], queryFn: listShops })
  const [creating, setCreating] = useState(false)
  const [signingOutAll, setSigningOutAll] = useState(false)
  const logoutAll = useLogoutAll()
  const logoutAllMutation = useMutation({ mutationFn: logoutAll })
  const [changingEmail, setChangingEmail] = useState(false)
  const [deletingAccount, setDeletingAccount] = useState(false)
  const [password, setPassword] = useState('')
  const deleteAccount = useDeleteAccount()
  const deleteMutation = useMutation({ mutationFn: () => deleteAccount(password) })

  return (
    <div className="shops-page">
      <header className="shops-head">
        <div>
          <h1>我的店家</h1>
          <p className="muted">你好,{session?.user.name}</p>
        </div>
        <div className="shops-head-actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => setSigningOutAll(true)}
          >
            登出所有裝置
          </button>
          <button type="button" className="btn btn-secondary" onClick={logout}>
            <LogOutIcon width={16} height={16} />
            登出
          </button>
        </div>
      </header>

      {session && !session.user.email_verified && (
        <VerifyEmailNotice
          email={session.user.email}
          onChangeEmail={() => setChangingEmail(true)}
        />
      )}

      {shops.isPending && <Loading label="載入店家…" />}
      {shops.isError && <ErrorState error={shops.error} onRetry={() => shops.refetch()} />}

      {shops.data && shops.data.length === 0 && (
        <div className="empty-card">
          <h2>還沒有店家</h2>
          <p className="muted">建立你的第一家店,設定服務與營業時間後,顧客就能線上預約。</p>
          <button type="button" className="btn btn-primary" onClick={() => setCreating(true)}>
            <PlusIcon width={16} height={16} />
            建立店家
          </button>
        </div>
      )}

      {shops.data && shops.data.length > 0 && (
        <>
          <ul className="shop-list">
            {shops.data.map((shop) => (
              <li key={shop.id}>
                <Link to={`/admin/${shop.slug}`} className="card shop-card">
                  <Avatar name={shop.name} size="md" />
                  <div className="shop-card-main">
                    <h2>{shop.name}</h2>
                    <p className="muted small">
                      預約頁 /s/{shop.slug} · {ROLE_LABEL[shop.role]}
                    </p>
                  </div>
                  <ArrowRightIcon width={18} height={18} className="shop-card-arrow" />
                </Link>
              </li>
            ))}
          </ul>
          <button type="button" className="btn btn-secondary" onClick={() => setCreating(true)}>
            <PlusIcon width={16} height={16} />
            建立另一家店
          </button>
        </>
      )}

      <CreateShopModal open={creating} onClose={() => setCreating(false)} />
      <AccountActivity />
      <ChangeEmailModal
        open={changingEmail}
        current={session?.user.email ?? ''}
        onClose={() => setChangingEmail(false)}
      />
      <p className="shops-foot">
        <button type="button" className="link-btn" onClick={() => setChangingEmail(true)}>
          更換 Email
        </button>
        <button type="button" className="link-btn" onClick={() => setDeletingAccount(true)}>
          刪除我的帳號
        </button>
      </p>
      <ConfirmDialog
        open={deletingAccount}
        title="刪除你的帳號?"
        message="帳號會被永久刪除,無法還原:你的姓名與 Email 會被抹除、所有裝置立刻登出,這個 Email 之後可以重新註冊。你曾經負責的歷史預約會保留,但不再顯示你的名字。如果你還是某家店的擁有者,要先刪除那家店;還有負責的未來預約也要先處理。"
        confirmLabel="永久刪除帳號"
        danger
        pending={deleteMutation.isPending}
        error={
          deleteMutation.error instanceof ApiError
            ? deleteMutation.error.message
            : deleteMutation.error
              ? '操作失敗,請再試一次。'
              : null
        }
        extra={
          <TextField label="輸入密碼確認">
            <input
              name="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </TextField>
        }
        onConfirm={() => deleteMutation.mutate()}
        onClose={() => {
          setDeletingAccount(false)
          setPassword('')
          deleteMutation.reset()
        }}
      />
      <ConfirmDialog
        open={signingOutAll}
        title="登出所有裝置?"
        message="包含這一台在內,所有裝置上的登入都會立刻失效,需要重新登入。懷疑帳號被別人用過時請這麼做(也建議一併更改密碼)。"
        confirmLabel="登出所有裝置"
        danger
        pending={logoutAllMutation.isPending}
        error={logoutAllMutation.isError ? '操作失敗,尚未登出,請再試一次。' : null}
        onConfirm={() => logoutAllMutation.mutate()}
        onClose={() => {
          setSigningOutAll(false)
          logoutAllMutation.reset()
        }}
      />
    </div>
  )
}

/** 還沒驗證 Email:提醒、可重寄,在別處點了連結後可以重新檢查 */
function VerifyEmailNotice({ email, onChangeEmail }: { email: string; onChangeEmail: () => void }) {
  const resend = useMutation({ mutationFn: resendVerification })
  const recheck = useMutation({ mutationFn: restoreSession })
  return (
    <div className="notice notice-warning verify-notice" role="status">
      <p>
        <strong>請驗證你的 Email。</strong>驗證信已寄到 {email},點信中的連結後才能建立店家。
      </p>
      {resend.isSuccess && <p>{resend.data.message}</p>}
      {resend.isError && (
        <p role="alert">
          {resend.error instanceof ApiError && resend.error.status === 429
            ? '這個小時已經寄了太多封,請稍後再試。'
            : resend.error instanceof ApiError
              ? resend.error.message
              : '寄送失敗,請再試一次。'}
        </p>
      )}
      <div className="verify-notice-actions">
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          disabled={resend.isPending}
          onClick={() => resend.mutate()}
        >
          重新寄送驗證信
        </button>
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          disabled={recheck.isPending}
          onClick={() => recheck.mutate()}
        >
          我已驗證,重新檢查
        </button>
        <button type="button" className="btn btn-secondary btn-sm" onClick={onChangeEmail}>
          Email 打錯了?更換
        </button>
      </div>
    </div>
  )
}

/** 更換 Email:重新輸入密碼,確認信寄到新地址,點了才換(目前的 Email 在那之前照常使用) */
function ChangeEmailModal({
  open,
  current,
  onClose,
}: {
  open: boolean
  current: string
  onClose: () => void
}) {
  const [values, setValues] = useState({ email: '', password: '' })
  const [emailError, setEmailError] = useState<string | null>(null)
  const mutation = useMutation({
    mutationFn: () => requestEmailChange(values.email.trim(), values.password),
  })
  const close = () => {
    setValues({ email: '', password: '' })
    setEmailError(null)
    mutation.reset()
    onClose()
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = validateEmail(values.email)
    setEmailError(found)
    if (!found) mutation.mutate()
  }

  return (
    <Modal open={open} title="更換 Email" onClose={close}>
      {mutation.isSuccess ? (
        <>
          <Notice tone="success">
            {mutation.data.message}在那之前請繼續用 {current} 登入。
          </Notice>
          <div className="actions">
            <button type="button" className="btn btn-primary" onClick={close}>
              知道了
            </button>
          </div>
        </>
      ) : (
        <>
          {mutation.error && (
            <Notice tone="error">
              {mutation.error instanceof ApiError && mutation.error.status === 429
                ? '這個小時已經申請太多次,請稍後再試。'
                : mutation.error instanceof ApiError
                  ? mutation.error.message
                  : '申請失敗,請再試一次。'}
            </Notice>
          )}
          <p className="muted small">
            目前是 {current}。確認信會寄到新的 Email,開啟信中的連結後才會更換。
          </p>
          <form className="form form-plain" onSubmit={submit} noValidate>
            <TextField label="新的 Email" error={emailError ?? undefined}>
              <input
                name="new_email"
                type="email"
                autoComplete="email"
                value={values.email}
                onChange={(e) => setValues({ ...values, email: e.target.value })}
                aria-invalid={!!emailError}
              />
            </TextField>
            <TextField label="輸入密碼確認">
              <input
                name="password"
                type="password"
                autoComplete="current-password"
                value={values.password}
                onChange={(e) => setValues({ ...values, password: e.target.value })}
              />
            </TextField>
            <div className="actions">
              <button
                type="submit"
                className="btn btn-primary"
                disabled={mutation.isPending || !values.password}
              >
                {mutation.isPending ? '寄送中…' : '寄送確認信'}
              </button>
              <button type="button" className="btn btn-secondary" onClick={close}>
                取消
              </button>
            </div>
          </form>
        </>
      )}
    </Modal>
  )
}

function CreateShopModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [values, setValues] = useState({ name: '', slug: '', timezone: 'Asia/Taipei' })
  const [errors, setErrors] = useState<{ name?: string; slug?: string }>({})

  const mutation = useMutation({
    mutationFn: () =>
      createShop({ name: values.name.trim(), slug: values.slug.trim(), timezone: values.timezone }),
    onSuccess: (shop) => {
      queryClient.invalidateQueries({ queryKey: ['admin', 'shops'] })
      onClose()
      navigate(`/admin/${shop.slug}`)
    },
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = {
      name: validateName(values.name, '店家名稱') ?? undefined,
      slug: validateSlug(values.slug) ?? undefined,
    }
    setErrors(found)
    if (!found.name && !found.slug) mutation.mutate()
  }

  return (
    <Modal open={open} title="建立店家" onClose={onClose}>
      {mutation.error && (
        <Notice tone="error">
          {mutation.error instanceof ApiError ? mutation.error.message : '建立失敗,請再試一次。'}
        </Notice>
      )}
      <form className="form form-plain" onSubmit={submit} noValidate>
        <TextField label="店家名稱" error={errors.name}>
          <input
            name="name"
            value={values.name}
            onChange={(e) => setValues({ ...values, name: e.target.value })}
            aria-invalid={!!errors.name}
          />
        </TextField>
        <TextField
          label="預約頁網址代稱"
          error={errors.slug}
          hint={`顧客的預約頁會是 ${window.location.origin}/s/${values.slug.trim() || '你的代稱'}`}
        >
          <input
            name="slug"
            value={values.slug}
            onChange={(e) => setValues({ ...values, slug: e.target.value.toLowerCase() })}
            placeholder="例如 my-salon"
            autoCapitalize="none"
            aria-invalid={!!errors.slug}
          />
        </TextField>
        <TextField
          label="店家所在時區"
          hint="營業時間與預約時段都以這個時區為準,之後可在店家的「設定」修改"
        >
          <select
            name="timezone"
            value={values.timezone}
            onChange={(e) => setValues({ ...values, timezone: e.target.value })}
          >
            {TIMEZONES.map((tz) => (
              <option key={tz.value} value={tz.value}>
                {tz.label}
              </option>
            ))}
          </select>
        </TextField>
        <div className="actions">
          <button type="submit" className="btn btn-primary" disabled={mutation.isPending}>
            {mutation.isPending ? '建立中…' : '建立店家'}
          </button>
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            取消
          </button>
        </div>
      </form>
    </Modal>
  )
}

const EVENT_LABEL: Record<AccountEvent['kind'], string> = {
  registered: '建立帳號',
  login: '登入',
  login_failed: '登入失敗(密碼錯誤)',
  locked: '連續失敗,帳號暫時鎖定',
  logout_all: '登出所有裝置',
  password_reset: '重設密碼',
  email_verified: '驗證 Email',
  email_changed: '更換 Email',
}

/** 最近的帳號活動:看到不是自己的登入或一直失敗的嘗試,就該改密碼 / 登出所有裝置 */
function AccountActivity() {
  // 帳號不屬於任何店家,沒有「店家時區」:用瀏覽器的時區顯示,並標明
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const events = useQuery({
    queryKey: ['admin', 'account-events'],
    queryFn: listAccountEvents,
  })
  return (
    <details className="account-activity">
      <summary>最近的帳號活動</summary>
      {events.isPending && <Loading label="載入中…" />}
      {events.isError && <ErrorState error={events.error} onRetry={() => events.refetch()} />}
      {events.data?.length === 0 && <p className="muted small">沒有紀錄。</p>}
      <ul className="rows">
        {events.data?.map((e, i) => (
          <li key={i} className="row">
            <div className="row-main">
              <div className="row-title">{EVENT_LABEL[e.kind] ?? e.kind}</div>
              <div className="row-sub">{formatDateTime(e.created_at, timeZone)}</div>
            </div>
          </li>
        ))}
      </ul>
      <p className="muted small">
        如果看到不是你做的登入或一直失敗的嘗試,請改密碼並「登出所有裝置」。時間以你的瀏覽器時區(
        {tzLabel(timeZone)})顯示。
      </p>
    </details>
  )
}
