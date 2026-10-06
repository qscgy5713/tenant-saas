import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Avatar } from '../../components/Avatar'
import { ArrowRightIcon, LogOutIcon, PlusIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import { useTitle } from '../../lib/useTitle'
import { validateName, validateSlug } from '../../lib/validate'
import { createShop, listShops, resendVerification } from '../api'
import { ConfirmDialog, Modal } from '../components/Modal'
import { TextField } from '../components/AuthCard'
import { useLogout, useLogoutAll } from '../logout'
import { TIMEZONES } from '../timezones'
import { ROLE_LABEL } from '../ShopContext'
import { restoreSession, useSession } from '../session'

export function ShopsPage() {
  useTitle('選擇店家 · 店家後台')
  const session = useSession()
  const logout = useLogout()
  const shops = useQuery({ queryKey: ['admin', 'shops'], queryFn: listShops })
  const [creating, setCreating] = useState(false)
  const [signingOutAll, setSigningOutAll] = useState(false)
  const logoutAll = useLogoutAll()
  const logoutAllMutation = useMutation({ mutationFn: logoutAll })

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

      {session && !session.user.email_verified && <VerifyEmailNotice email={session.user.email} />}

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
function VerifyEmailNotice({ email }: { email: string }) {
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
      </div>
    </div>
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
