import { useMutation, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'
import { ErrorState, Loading, Notice } from '../../components/States'
import { meterStep } from '../../lib/meter'
import { formatPrice } from '../../lib/money'
import { ApiError } from '../../api/client'
import { redirectTo } from '../../lib/redirect'
import { getBilling, getPlan, listPlans, openPortal, startCheckout } from '../api'
import { useShop } from '../ShopContext'
import type { Billing, PlanInfo } from '../types'

export function PlanPage() {
  const { slug, isOwner } = useShop()
  const [params] = useSearchParams()
  const returned = params.get('checkout') // Stripe 結帳完成 / 取消後導回來
  const current = useQuery({
    queryKey: ['admin', 'plan', slug],
    queryFn: () => getPlan(slug),
    staleTime: 10_000,
    // 剛付完款時,方案是等 Stripe 的 webhook 才會生效(不信任導回來的網址參數),所以每 2 秒查一次,最多 15 次
    refetchInterval: (q) =>
      returned === 'success' && q.state.data?.plan.id === 'free' && q.state.dataUpdateCount < 15
        ? 2000
        : false,
  })
  // 付款只有店主能操作;其他人看不到,也不必查
  const billing = useQuery({
    queryKey: ['admin', 'billing', slug],
    queryFn: () => getBilling(slug),
    enabled: isOwner,
    staleTime: 10_000,
  })
  const paying = billing.data?.enabled === true
  const plans = useQuery({ queryKey: ['plans'], queryFn: listPlans, staleTime: 5 * 60_000 })

  if (current.isPending) return <Loading />
  if (current.isError) return <ErrorState error={current.error} onRetry={() => current.refetch()} />

  const { plan, usage } = current.data
  // 待接受的邀請也算名額(邀請會預留位子)
  const staffUsed = usage.staff + usage.pending_invitations

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">方案與用量</h1>
          <p className="muted small">
            目前方案:<strong>{plan.name}</strong>({formatPrice(plan.price_cents)})
          </p>
        </div>
      </div>

      <section className="usage card-section">
        <h2>用量</h2>
        <Meter
          label="成員"
          detail={`含 ${usage.pending_invitations} 個待接受的邀請`}
          used={staffUsed}
          max={plan.max_staff}
        />
        <Meter label="啟用中的服務" used={usage.services} max={plan.max_services} />
        <Meter
          label="本月預約"
          detail="依店家當地月份計算,已取消與待確認的不算"
          used={usage.bookings_this_month}
          max={plan.max_bookings_per_month}
        />
      </section>

      <section className="section">
        <h2>所有方案</h2>
        {isOwner && billing.data && !paying && (
          <Notice tone="info">
            線上升級與付款尚未開放。需要更高的方案請聯絡平台管理員,升級後會立刻生效。
          </Notice>
        )}
        {!isOwner && <Notice tone="info">升級或變更方案請聯絡店家擁有者。</Notice>}
        {returned === 'success' && (
          <Notice tone="success">
            {plan.id === 'free'
              ? '付款已送出,方案正在更新(通常幾秒內完成)。若稍後仍未生效,請聯絡我們。'
              : `已升級為「${plan.name}」。`}
          </Notice>
        )}
        {returned === 'cancel' && <Notice tone="info">已取消付款,方案沒有變更。</Notice>}
        {paying && billing.data && <SubscriptionBar billing={billing.data} slug={slug} />}
        {plans.isPending && <Loading label="載入方案…" />}
        {plans.isError && <ErrorState error={plans.error} onRetry={() => plans.refetch()} />}
        <ul className="plan-grid">
          {plans.data?.map((p) => (
            <PlanCard
              key={p.id}
              plan={p}
              active={p.id === plan.id}
              billing={isOwner ? billing.data : undefined}
              slug={slug}
            />
          ))}
        </ul>
      </section>
    </div>
  )
}

/** 用量條。max 為 null 代表不限。到 80% 變黃、滿了變紅 */
function Meter({
  label,
  detail,
  used,
  max,
}: {
  label: string
  detail?: string
  used: number
  max: number | null
}) {
  // 寬度用 class(w-0 … w-100,每 5% 一格)而不是行內 style:正式環境的 CSP 是 style-src 'self',行內樣式會被擋掉
  const ratio = max ? Math.min(1, used / max) : 0
  const tone = max === null ? 'ok' : used >= max ? 'full' : ratio >= 0.8 ? 'warn' : 'ok'
  return (
    <div className="meter">
      <div className="meter-head">
        <strong>{label}</strong>
        <span className={`meter-num meter-${tone}`}>
          {used} / {max ?? '不限'}
        </span>
      </div>
      {max !== null && (
        <div
          className="meter-track"
          role="meter"
          aria-label={label}
          aria-valuemin={0}
          aria-valuemax={max}
          aria-valuenow={Math.min(used, max)}
        >
          <div className={`meter-fill meter-fill-${tone} w-${meterStep(used, max ?? 0)}`} />
        </div>
      )}
      {detail && <p className="muted small">{detail}</p>}
      {tone === 'full' && <p className="meter-msg">已達上限,無法再新增。</p>}
    </div>
  )
}

const limit = (n: number | null, unit: string) => (n === null ? `${unit}不限` : `${unit} ${n}`)

function describeStatus(billing: Billing): string | null {
  const s = billing.subscription
  if (!s) return null
  const end = s.current_period_end
    ? new Date(s.current_period_end).toLocaleDateString('zh-TW')
    : null
  if (s.status === 'past_due') return '付款失敗,請更新付款方式,否則訂閱將被取消。'
  if (s.cancel_at_period_end)
    return end ? `訂閱已設定為 ${end} 到期後取消。` : '訂閱已設定為到期後取消。'
  if (s.status === 'active' || s.status === 'trialing')
    return end ? `訂閱中,下次續訂日 ${end}。` : '訂閱中。'
  return null
}

function useRedirect(call: () => Promise<{ url: string }>) {
  return useMutation({ mutationFn: call, onSuccess: ({ url }) => redirectTo(url) })
}

const errorText = (e: unknown) => (e instanceof ApiError ? e.message : '操作失敗,請再試一次。')

/** 訂閱狀態與「管理訂閱」(變更方案、更新付款方式、取消 —— 都在 Stripe 的客戶入口) */
function SubscriptionBar({ billing, slug }: { billing: Billing; slug: string }) {
  const portal = useRedirect(() => openPortal(slug))
  const text = describeStatus(billing)
  if (!text && !billing.can_manage) return null
  return (
    <div className="card-section">
      {text && <p>{text}</p>}
      {billing.can_manage && (
        <button
          type="button"
          className="btn btn-secondary"
          disabled={portal.isPending}
          onClick={() => portal.mutate()}
        >
          {portal.isPending ? '前往中…' : '管理訂閱與付款方式'}
        </button>
      )}
      {portal.isError && <Notice tone="error">{errorText(portal.error)}</Notice>}
    </div>
  )
}

function PlanCard({
  plan,
  active,
  billing,
  slug,
}: {
  plan: PlanInfo
  active: boolean
  billing?: Billing
  slug: string
}) {
  const checkout = useRedirect(() => startCheckout(slug, plan.id))
  const canBuy =
    !active && !!billing?.enabled && billing.can_checkout && billing.purchasable.includes(plan.id)
  return (
    <li className={`card plan-card ${active ? 'plan-on' : ''}`}>
      <div className="plan-head">
        <h3>{plan.name}</h3>
        {active && <span className="chip chip-staff">目前方案</span>}
      </div>
      <p className="plan-price">{formatPrice(plan.price_cents)}</p>
      <ul className="plan-limits">
        <li>{limit(plan.max_staff, '成員')}</li>
        <li>{limit(plan.max_services, '服務')}</li>
        <li>{limit(plan.max_bookings_per_month, '每月預約')}</li>
      </ul>
      {canBuy && (
        <button
          type="button"
          className="btn btn-primary btn-block"
          disabled={checkout.isPending}
          onClick={() => checkout.mutate()}
        >
          {checkout.isPending ? '前往付款頁…' : `升級到${plan.name}`}
        </button>
      )}
      {checkout.isError && <Notice tone="error">{errorText(checkout.error)}</Notice>}
    </li>
  )
}
