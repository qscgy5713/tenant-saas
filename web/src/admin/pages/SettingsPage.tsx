import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validateName } from '../../lib/validate'
import { updateShop } from '../api'
import { TextField } from '../components/AuthCard'
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
    </div>
  )
}
