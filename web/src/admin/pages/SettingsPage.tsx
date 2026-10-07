import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validateName } from '../../lib/validate'
import {
  cancelShopDeletion,
  requestShopDeletion,
  setDataRetention,
  setShopProfile,
  updateShop,
} from '../api'
import { TextField } from '../components/AuthCard'
import { ConfirmDialog } from '../components/Modal'
import { formatDateTime } from '../../lib/time'
import { useShop } from '../ShopContext'
import { TIMEZONES } from '../timezones'

/** 店家設定(僅店主)。網址代稱不能改:顧客手上的預約連結與書籤都靠它 */
export function SettingsPage() {
  const { slug, shop, isOwner } = useShop()
  const queryClient = useQueryClient()
  const [name, setName] = useState(shop.name)
  const [timezone, setTimezone] = useState(shop.timezone)
  const [nameError, setNameError] = useState<string>()
  const [saved, setSaved] = useState(false)

  const mutation = useMutation({
    mutationFn: () =>
      updateShop(slug, {
        ...(name.trim() !== shop.name ? { name: name.trim() } : {}),
        ...(timezone !== shop.timezone ? { timezone } : {}),
      }),
    onSuccess: () => {
      setSaved(true)
      // 時區改了:所有以店家時區顯示的東西都要重抓
      queryClient.invalidateQueries({ queryKey: ['admin', 'shop', slug] })
      queryClient.invalidateQueries({ queryKey: ['admin', 'shops'] })
      queryClient.invalidateQueries({ queryKey: ['admin', 'bookings', slug] })
      queryClient.invalidateQueries({ queryKey: ['availability'] })
    },
  })

  if (!isOwner) {
    return (
      <div className="state">
        <h1>沒有權限</h1>
        <p>店家設定只有擁有者可以修改。</p>
        <Link className="btn btn-secondary" to={`/admin/${slug}`}>
          回到總覽
        </Link>
      </div>
    )
  }

  const changed = name.trim() !== shop.name || timezone !== shop.timezone
  const tzChanging = timezone !== shop.timezone

  const submit = (e: FormEvent) => {
    e.preventDefault()
    setSaved(false)
    const err = validateName(name, '店家名稱') ?? undefined
    setNameError(err)
    if (!err && changed) mutation.mutate()
  }

  const err = mutation.error
  const message = err instanceof ApiError ? err.message : err ? '儲存失敗,請再試一次。' : null
  // 目前的時區不在常用清單時(例如由其他管道建立),也要能顯示出來
  const options = TIMEZONES.some((t) => t.value === shop.timezone)
    ? TIMEZONES
    : [{ value: shop.timezone, label: shop.timezone }, ...TIMEZONES]

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">店家設定</h1>
          <p className="muted small">
            預約頁網址:{window.location.origin}/s/{slug}(代稱無法更改)
          </p>
        </div>
      </div>
      <form className="form card-section" onSubmit={submit} noValidate>
        {saved && <Notice tone="success">已儲存。</Notice>}
        {message && <Notice tone="error">{message}</Notice>}
        <TextField label="店家名稱" error={nameError}>
          <input
            name="name"
            value={name}
            onChange={(e) => {
              setName(e.target.value)
              setSaved(false)
            }}
            aria-invalid={!!nameError}
          />
        </TextField>
        <TextField label="店家所在時區">
          <select
            name="timezone"
            value={timezone}
            onChange={(e) => {
              setTimezone(e.target.value)
              setSaved(false)
            }}
          >
            {options.map((tz) => (
              <option key={tz.value} value={tz.value}>
                {tz.label}
              </option>
            ))}
          </select>
        </TextField>
        {tzChanging && (
          <Notice tone="warning">
            改時區<strong>不會移動</strong>
            既有的預約與休假(它們是絕對時間),但每週的營業時間是「店家當地時刻」,
            會改用新時區解讀。改完請到「團隊」確認每位成員的營業時間。
          </Notice>
        )}
        <div className="actions">
          <button
            type="submit"
            className="btn btn-primary"
            disabled={!changed || mutation.isPending}
          >
            {mutation.isPending ? '儲存中…' : '儲存'}
          </button>
        </div>
      </form>
      <ProfileSection />
      <RetentionSection />
      <DeletionSection />
    </div>
  )
}

const errorText = (e: unknown) =>
  e instanceof ApiError ? e.message : e ? '操作失敗,請再試一次。' : null

/** 顧客預約頁上顯示的店家資訊(都是選填)。電話只能是數字與 + - ( ) # 空白:預約頁會把它做成撥號連結 */
function ProfileSection() {
  const { slug, shop } = useShop()
  const queryClient = useQueryClient()
  const [values, setValues] = useState({
    description: shop.description ?? '',
    address: shop.address ?? '',
    phone: shop.phone ?? '',
  })
  const [saved, setSaved] = useState(false)
  const errors = {
    description: values.description.length > 500 ? '最多 500 字' : undefined,
    address: values.address.length > 200 ? '最多 200 字' : undefined,
    phone: !/^[0-9+\-()# ]*$/.test(values.phone)
      ? '只能包含數字與 + - ( ) # 空白'
      : values.phone.length > 30
        ? '最多 30 字'
        : undefined,
  }
  const valid = !errors.description && !errors.address && !errors.phone
  const changed =
    values.description.trim() !== (shop.description ?? '') ||
    values.address.trim() !== (shop.address ?? '') ||
    values.phone.trim() !== (shop.phone ?? '')
  const mutation = useMutation({
    mutationFn: () => setShopProfile(slug, values),
    onSuccess: () => {
      setSaved(true)
      queryClient.invalidateQueries({ queryKey: ['admin', 'shop', slug] })
      queryClient.invalidateQueries({ queryKey: ['shop', slug] })
    },
  })
  const set =
    (key: keyof typeof values) =>
    (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setValues((v) => ({ ...v, [key]: e.target.value }))
      setSaved(false)
    }
  return (
    <form
      className="form card-section"
      onSubmit={(e) => {
        e.preventDefault()
        setSaved(false)
        if (valid && changed) mutation.mutate()
      }}
      noValidate
    >
      <h2>店家資訊</h2>
      <p className="muted small">顯示在顧客的預約頁上,全部選填。清空就是不顯示。</p>
      {saved && <Notice tone="success">已儲存。</Notice>}
      {mutation.error && <Notice tone="error">{errorText(mutation.error)}</Notice>}
      <TextField label="店家簡介" error={errors.description} hint="最多 500 字,可以換行">
        <textarea
          name="description"
          rows={4}
          value={values.description}
          onChange={set('description')}
          aria-invalid={!!errors.description}
        />
      </TextField>
      <TextField label="地址" error={errors.address}>
        <input
          name="address"
          value={values.address}
          onChange={set('address')}
          aria-invalid={!!errors.address}
        />
      </TextField>
      <TextField label="電話" error={errors.phone}>
        <input
          name="phone"
          inputMode="tel"
          value={values.phone}
          onChange={set('phone')}
          aria-invalid={!!errors.phone}
        />
      </TextField>
      <div className="actions">
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!valid || !changed || mutation.isPending}
        >
          {mutation.isPending ? '儲存中…' : '儲存'}
        </button>
      </div>
    </form>
  )
}

/** 顧客個資到期自動匿名化:姓名 / Email / 電話被抹除,預約紀錄與統計保留 */
function RetentionSection() {
  const { slug, shop } = useShop()
  const queryClient = useQueryClient()
  const [days, setDays] = useState(String(shop.customer_retention_days))
  const [saved, setSaved] = useState(false)
  const n = Number(days)
  const valid = Number.isInteger(n) && n >= 90 && n <= 3650
  const mutation = useMutation({
    mutationFn: () => setDataRetention(slug, n),
    onSuccess: () => {
      setSaved(true)
      queryClient.invalidateQueries({ queryKey: ['admin', 'shop', slug] })
    },
  })
  return (
    <form
      className="form card-section"
      onSubmit={(e) => {
        e.preventDefault()
        setSaved(false)
        if (valid) mutation.mutate()
      }}
      noValidate
    >
      <h2>顧客資料保留</h2>
      <p className="muted small">
        顧客在最後一筆預約之後,個資(姓名、Email、電話、預約備註)會保留這麼久,到期自動匿名化;預約紀錄與統計會保留。
        已經匿名化的資料無法還原。
      </p>
      {saved && <Notice tone="success">已儲存。</Notice>}
      {mutation.error && <Notice tone="error">{errorText(mutation.error)}</Notice>}
      <TextField
        label="保留天數"
        error={valid ? undefined : '請輸入 90 到 3650 之間的整數(約 3 個月到 10 年)'}
        hint="預設 730 天(2 年)。縮短只影響之後的清理,也會套用在已經超過新天數的顧客。"
      >
        <input
          name="customer_retention_days"
          inputMode="numeric"
          value={days}
          onChange={(e) => {
            setDays(e.target.value)
            setSaved(false)
          }}
          aria-invalid={!valid}
        />
      </TextField>
      <div className="actions">
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!valid || n === shop.customer_retention_days || mutation.isPending}
        >
          {mutation.isPending ? '儲存中…' : '儲存'}
        </button>
      </div>
    </form>
  )
}

/** 刪除店家:30 天寬限,期間公開預約頁關閉、可取消;期滿連同所有資料永久刪除 */
function DeletionSection() {
  const { slug, shop } = useShop()
  const queryClient = useQueryClient()
  const [asking, setAsking] = useState(false)
  const [typed, setTyped] = useState('')
  const [mismatch, setMismatch] = useState(false)
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['admin', 'shop', slug] })
  const request = useMutation({
    mutationFn: () => requestShopDeletion(slug, typed.trim()),
    onSuccess: () => {
      setAsking(false)
      setTyped('')
      refresh()
    },
  })
  const cancel = useMutation({ mutationFn: () => cancelShopDeletion(slug), onSuccess: refresh })
  const close = () => {
    setAsking(false)
    setTyped('')
    setMismatch(false)
    request.reset()
  }

  return (
    <section className="card-section danger-zone">
      <h2>刪除店家</h2>
      {shop.deletion_scheduled_at ? (
        <>
          <Notice tone="warning">
            已申請刪除:公開預約頁已關閉,
            {formatDateTime(shop.deletion_scheduled_at, shop.timezone)}
            之後會連同所有資料(服務、預約、顧客、稽核紀錄)永久刪除。
          </Notice>
          {cancel.error && <Notice tone="error">{errorText(cancel.error)}</Notice>}
          <button
            type="button"
            className="btn btn-secondary"
            disabled={cancel.isPending}
            onClick={() => cancel.mutate()}
          >
            取消刪除
          </button>
        </>
      ) : (
        <>
          <p className="muted small">
            申請後有 30
            天寬限:公開預約頁立即關閉,期間可以取消;期滿後店家與所有資料會永久刪除,無法還原。
            需要先取消所有未來的預約,並取消進行中的訂閱。
          </p>
          <button type="button" className="btn btn-danger" onClick={() => setAsking(true)}>
            刪除這家店
          </button>
        </>
      )}
      <ConfirmDialog
        open={asking}
        title="刪除這家店?"
        message={`輸入網址代稱「${slug}」確認。公開預約頁會立即關閉,30 天後所有資料會永久刪除。`}
        confirmLabel="申請刪除"
        danger
        pending={request.isPending}
        error={mismatch ? '輸入的網址代稱不符' : errorText(request.error)}
        extra={
          <TextField label="網址代稱">
            <input
              name="confirm_slug"
              value={typed}
              autoCapitalize="none"
              onChange={(e) => {
                setTyped(e.target.value)
                setMismatch(false)
              }}
            />
          </TextField>
        }
        onConfirm={() => {
          if (typed.trim() !== slug) return setMismatch(true)
          request.mutate()
        }}
        onClose={close}
      />
    </section>
  )
}
