import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Avatar } from '../../components/Avatar'
import { PlusIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import { formatDateTime } from '../../lib/time'
import { validateEmail } from '../../lib/validate'
import {
  changeRole,
  createInvitation,
  listInvitations,
  removeMember,
  revokeInvitation,
} from '../api'
import { ConfirmDialog, Modal } from '../components/Modal'
import { canRemove } from '../permissions'
import { useMembers } from '../queries'
import { ROLE_LABEL, useShop } from '../ShopContext'
import type { Member, Role } from '../types'

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? '操作失敗,請再試一次。' : null

export function TeamPage() {
  const { slug, user, role, canManage, isOwner, shop } = useShop()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const members = useMembers(slug)
  const invitations = useQuery({
    queryKey: ['admin', 'invitations', slug],
    queryFn: () => listInvitations(slug),
    enabled: canManage,
  })
  const [inviting, setInviting] = useState(false)
  const [removing, setRemoving] = useState<Member | null>(null)
  const [revoking, setRevoking] = useState<string | null>(null)

  const refreshMembers = () => {
    queryClient.invalidateQueries({ queryKey: ['admin', 'members', slug] })
    queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
  }

  const roleMutation = useMutation({
    mutationFn: (v: { userId: string; role: Exclude<Role, 'owner'> }) =>
      changeRole(slug, v.userId, v.role),
    onSuccess: refreshMembers,
  })
  const removeMutation = useMutation({
    mutationFn: (m: Member) => removeMember(slug, m.user_id),
    onSuccess: (_d, m) => {
      setRemoving(null)
      refreshMembers()
      if (m.user_id === user.id) {
        // 自己退出:這家店已經進不去了,清掉它的快取並回到店家列表
        queryClient.removeQueries({ queryKey: ['admin'] })
        navigate('/admin')
      }
    },
  })
  const revokeMutation = useMutation({
    mutationFn: (id: string) => revokeInvitation(slug, id),
    onSuccess: () => {
      setRevoking(null)
      queryClient.invalidateQueries({ queryKey: ['admin', 'invitations', slug] })
      queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
    },
  })

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">團隊</h1>
          <p className="muted small">
            成員可以登入後台、接受預約。營業時間與可提供的服務在每位成員的設定頁。
          </p>
        </div>
        {canManage && (
          <div className="page-actions">
            <button type="button" className="btn btn-primary" onClick={() => setInviting(true)}>
              <PlusIcon width={16} height={16} />
              邀請成員
            </button>
          </div>
        )}
      </div>

      {roleMutation.error && <Notice tone="error">{errorText(roleMutation.error)}</Notice>}

      {members.isPending && <Loading label="載入成員…" />}
      {members.isError && <ErrorState error={members.error} onRetry={() => members.refetch()} />}

      <ul className="rows">
        {members.data?.map((m) => {
          const self = m.user_id === user.id
          return (
            <li key={m.user_id} className="row member-row">
              <div className="member-id">
                <Avatar name={m.name} size="md" />
                <div className="row-main">
                  <div className="row-title">
                    {m.name}
                    {self && <span className="chip chip-muted badge-inline">你</span>}
                  </div>
                  <div className="row-sub">{m.email}</div>
                </div>
              </div>
              <div className="row-actions">
                {isOwner && m.role !== 'owner' ? (
                  <select
                    aria-label={`${m.name} 的角色`}
                    value={m.role}
                    disabled={roleMutation.isPending}
                    onChange={(e) =>
                      roleMutation.mutate({
                        userId: m.user_id,
                        role: e.target.value as 'manager' | 'staff',
                      })
                    }
                  >
                    <option value="manager">{ROLE_LABEL.manager}</option>
                    <option value="staff">{ROLE_LABEL.staff}</option>
                  </select>
                ) : (
                  <span className={`chip chip-${m.role}`}>{ROLE_LABEL[m.role]}</span>
                )}
                <Link className="btn btn-secondary btn-sm" to={m.user_id}>
                  {self || canManage ? '設定' : '查看'}
                </Link>
                {canRemove({ id: user.id, role }, m) && (
                  <button
                    type="button"
                    className="btn btn-danger-outline btn-sm"
                    onClick={() => setRemoving(m)}
                  >
                    {self ? '退出團隊' : '移除'}
                  </button>
                )}
              </div>
            </li>
          )
        })}
      </ul>

      {canManage && (
        <section className="section">
          <h2>待接受的邀請</h2>
          {invitations.isPending && <Loading label="載入邀請…" />}
          {invitations.isError && (
            <ErrorState error={invitations.error} onRetry={() => invitations.refetch()} />
          )}
          {invitations.data?.length === 0 && <p className="muted">目前沒有待接受的邀請。</p>}
          {revokeMutation.error && <Notice tone="error">{errorText(revokeMutation.error)}</Notice>}
          <ul className="rows">
            {invitations.data?.map((inv) => (
              <li key={inv.id} className="row">
                <div className="row-main">
                  <div className="row-title">{inv.email}</div>
                  <div className="row-sub">
                    以「{ROLE_LABEL[inv.role]}」身分邀請 ·{' '}
                    {formatDateTime(inv.expires_at, shop.timezone)} 前有效
                  </div>
                </div>
                <div className="row-actions">
                  {(isOwner || inv.role === 'staff') && (
                    <button
                      type="button"
                      className="btn btn-danger-outline btn-sm"
                      onClick={() => setRevoking(inv.id)}
                    >
                      撤銷
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      <InviteModal open={inviting} onClose={() => setInviting(false)} />

      <ConfirmDialog
        open={removing !== null}
        title={removing?.user_id === user.id ? '退出這家店?' : '移除這位成員?'}
        message={
          removing?.user_id === user.id
            ? '退出後你就不能再進入這家店的後台。'
            : `${removing?.name} 會失去這家店的所有存取權限。如果他有預約紀錄就無法移除。`
        }
        confirmLabel={removing?.user_id === user.id ? '退出' : '移除'}
        danger
        pending={removeMutation.isPending}
        error={errorText(removeMutation.error)}
        onConfirm={() => removing && removeMutation.mutate(removing)}
        onClose={() => {
          setRemoving(null)
          removeMutation.reset()
        }}
      />
      <ConfirmDialog
        open={revoking !== null}
        title="撤銷這個邀請?"
        message="對方收到的邀請連結會立刻失效。"
        confirmLabel="撤銷"
        danger
        pending={revokeMutation.isPending}
        error={null}
        onConfirm={() => revoking && revokeMutation.mutate(revoking)}
        onClose={() => setRevoking(null)}
      />
    </div>
  )
}

function InviteModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Modal open={open} title="邀請成員" onClose={onClose}>
      <InviteForm onClose={onClose} />
    </Modal>
  )
}

function InviteForm({ onClose }: { onClose: () => void }) {
  const { slug, isOwner } = useShop()
  const queryClient = useQueryClient()
  const [email, setEmail] = useState('')
  const [role, setRole] = useState<'manager' | 'staff'>('staff')
  const [emailError, setEmailError] = useState<string | null>(null)

  const mutation = useMutation({
    mutationFn: () => createInvitation(slug, email.trim(), role),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin', 'invitations', slug] })
      queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
    },
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = validateEmail(email)
    setEmailError(found)
    if (!found) mutation.mutate()
  }

  if (mutation.data) {
    const link = `${window.location.origin}/invitations/accept#token=${mutation.data.token}`
    return (
      <>
        <Notice tone="success">
          已寄出邀請信到 <strong>{mutation.data.email}</strong>,連結 7 天內有效。
        </Notice>
        <p className="muted small">
          如果對方沒收到信,也可以把下面的連結直接傳給他。<strong>這個連結只會顯示這一次</strong>
          ,而且只有用被邀請的 Email 登入才能使用。
        </p>
        <input
          className="copy-field"
          readOnly
          value={link}
          onFocus={(e) => e.currentTarget.select()}
          aria-label="邀請連結"
        />
        <div className="actions">
          <button type="button" className="btn btn-primary" onClick={onClose}>
            完成
          </button>
        </div>
      </>
    )
  }

  return (
    <form className="form-plain new-booking" onSubmit={submit} noValidate>
      {mutation.error && <Notice tone="error">{errorText(mutation.error)}</Notice>}
      <div className={`field ${emailError ? 'field-error' : ''}`}>
        <label>
          <span className="field-label">對方的 Email</span>
          <input
            name="email"
            type="email"
            value={email}
            onChange={(e) => {
              setEmail(e.target.value)
              setEmailError(null)
            }}
            aria-invalid={!!emailError}
          />
        </label>
        {emailError && <p className="field-msg">{emailError}</p>}
      </div>
      <div className="field">
        <label>
          <span className="field-label">角色</span>
          <select
            name="role"
            value={role}
            onChange={(e) => setRole(e.target.value as 'manager' | 'staff')}
          >
            <option value="staff">{ROLE_LABEL.staff}(管理自己的行程)</option>
            {isOwner && (
              <option value="manager">{ROLE_LABEL.manager}(可管理服務、成員與所有預約)</option>
            )}
          </select>
        </label>
      </div>
      <div className="actions">
        <button type="submit" className="btn btn-primary" disabled={mutation.isPending}>
          {mutation.isPending ? '寄送中…' : '寄出邀請'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </form>
  )
}
