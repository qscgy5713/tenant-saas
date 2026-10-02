import { useQuery } from '@tanstack/react-query'
import { ErrorState, Loading, Notice } from '../../components/States'
import { meterStep } from '../../lib/meter'
import { formatPrice } from '../../lib/money'
import { getPlan, listPlans } from '../api'
import { useShop } from '../ShopContext'
import type { PlanInfo } from '../types'

export function PlanPage() {
  const { slug } = useShop()
  const current = useQuery({
    queryKey: ['admin', 'plan', slug],
    queryFn: () => getPlan(slug),
    staleTime: 10_000,
  })
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
        <Notice tone="info">
          線上升級與付款尚未開放。需要更高的方案請聯絡平台管理員,升級後會立刻生效。
        </Notice>
        {plans.isPending && <Loading label="載入方案…" />}
        {plans.isError && <ErrorState error={plans.error} onRetry={() => plans.refetch()} />}
        <ul className="plan-grid">
          {plans.data?.map((p) => (
            <PlanCard key={p.id} plan={p} active={p.id === plan.id} />
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

function PlanCard({ plan, active }: { plan: PlanInfo; active: boolean }) {
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
    </li>
  )
}
