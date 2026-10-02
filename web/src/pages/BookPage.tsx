import { useQueryClient, useMutation } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { ApiError } from '../api/client'
import { useServices, useShop, useStaff } from '../api/queries'
import { createBooking } from '../api/public'
import type { BookingRequested } from '../api/types'
import { DetailsForm } from '../components/DetailsForm'
import { CheckIcon, MailIcon } from '../components/Icons'
import { Shell } from '../components/Shell'
import { SlotPicker } from '../components/SlotPicker'
import { StaffPicker } from '../components/StaffPicker'
import { ErrorState, Loading, Notice } from '../components/States'
import { SummaryCard } from '../components/SummaryCard'
import type { TimeOption } from '../lib/slots'
import type { CustomerForm } from '../lib/validate'
import { formatDateTime } from '../lib/time'

type Step = 'time' | 'details' | 'sent'

export function BookPage() {
  const { slug = '', serviceId = '' } = useParams()
  const queryClient = useQueryClient()
  const shop = useShop(slug)
  const services = useServices(slug)
  const staff = useStaff(slug, serviceId)

  const [step, setStep] = useState<Step>('time')
  const [staffId, setStaffId] = useState<string | null>(null)
  const [choice, setChoice] = useState<TimeOption | null>(null)
  const [sent, setSent] = useState<{ email: string; booking: BookingRequested } | null>(null)

  const submit = useMutation({
    mutationFn: (customer: CustomerForm) =>
      createBooking(slug, {
        service_id: serviceId,
        staff_id: staffId ?? undefined,
        start: choice!.start,
        customer: {
          name: customer.name.trim(),
          email: customer.email.trim(),
          phone: customer.phone.trim() || undefined,
        },
      }).then((booking) => ({ booking, email: customer.email.trim() })),
    onSuccess: (result) => {
      setSent(result)
      setStep('sent')
    },
  })

  if (shop.isPending || services.isPending)
    return (
      <Shell>
        <Loading />
      </Shell>
    )
  if (shop.isError || services.isError) {
    return (
      <Shell>
        <ErrorState
          error={shop.error ?? services.error}
          onRetry={() => {
            shop.refetch()
            services.refetch()
          }}
        />
      </Shell>
    )
  }

  const service = services.data.find((s) => s.id === serviceId)
  if (!service) {
    return (
      <Shell title="找不到服務" shopName={shop.data.name} shopSlug={slug}>
        <div className="state">
          <h1>找不到這項服務</h1>
          <p>它可能已經下架。</p>
          <Link className="btn btn-secondary" to={`/s/${slug}`}>
            回到服務列表
          </Link>
        </div>
      </Shell>
    )
  }

  const timezone = shop.data.timezone
  const staffList = staff.data ?? []
  const staffName = (id: string) => staffList.find((s) => s.id === id)?.name
  const staffLabel = staffId
    ? (staffName(staffId) ?? '指定人員')
    : staffList.length === 1
      ? staffList[0].name
      : '不指定(由店家安排)'

  const backToTime = () => {
    // 時段可能已被別人訂走:回到選時間前先讓快取失效,重新取得最新的
    queryClient.invalidateQueries({ queryKey: ['availability', slug] })
    setChoice(null)
    submit.reset()
    setStep('time')
  }

  const conflict = submit.error instanceof ApiError && submit.error.status === 409

  return (
    <Shell title={service.name} shopName={shop.data.name} shopSlug={slug} wide>
      <nav className="crumbs" aria-label="導覽路徑">
        <Link to={`/s/${slug}`}>所有服務</Link>
        <span aria-hidden="true">/</span>
        <span aria-current="page">{service.name}</span>
      </nav>

      <Stepper current={step} />

      <div className="flow">
        <div className="flow-main">
          {step === 'time' && (
            <>
              <h1 className="page-title">選擇日期與時間</h1>
              {staff.isError && (
                <Notice tone="warning">無法載入服務人員名單,仍可選擇「不指定」。</Notice>
              )}
              {staffList.length > 1 && (
                <StaffPicker
                  staff={staffList}
                  value={staffId}
                  onChange={(id) => {
                    setStaffId(id)
                    setChoice(null)
                  }}
                />
              )}
              <SlotPicker
                key={staffId ?? 'any'}
                slug={slug}
                serviceId={serviceId}
                timezone={timezone}
                staffId={staffId}
                selectedStart={choice?.start ?? null}
                onSelect={setChoice}
              />
              <div className="cta-bar">
                <button
                  type="button"
                  className="btn btn-primary btn-block"
                  disabled={!choice}
                  onClick={() => setStep('details')}
                >
                  {choice
                    ? `下一步:填寫資料(${formatDateTime(choice.start, timezone)})`
                    : '請先選擇時間'}
                </button>
              </div>
            </>
          )}

          {step === 'details' && choice && (
            <>
              <h1 className="page-title">填寫聯絡資料</h1>
              {submit.isError &&
                (conflict ? (
                  <Notice tone="error">
                    {submit.error.message}
                    <button type="button" className="link-btn" onClick={backToTime}>
                      重新選擇時間
                    </button>
                  </Notice>
                ) : (
                  <Notice tone="error">
                    {submit.error instanceof ApiError
                      ? submit.error.message
                      : '送出失敗,請再試一次。'}
                  </Notice>
                ))}
              <DetailsForm pending={submit.isPending} onSubmit={(v) => submit.mutate(v)} />
              <button type="button" className="link-btn" onClick={backToTime}>
                ‹ 修改時間
              </button>
            </>
          )}

          {step === 'sent' && sent && (
            <div className="sent">
              <span className="sent-icon">
                <MailIcon width={28} height={28} />
              </span>
              <h1 className="page-title">請到信箱確認預約</h1>
              <Notice tone="warning">
                <strong>預約尚未成立。</strong>時段在你確認之前不會保留。
              </Notice>
              <p>
                我們已寄出確認信到 <strong>{sent.email}</strong>。請在 <strong>24 小時內</strong>
                開啟信中的連結,並按下「確認預約」。
              </p>
              <ul className="tips">
                <li>沒收到的話,請看一下垃圾郵件匣。</li>
                <li>超過 24 小時沒確認,這筆申請會自動失效,需要重新預約。</li>
                <li>確認前別人仍可能訂走同一個時段。</li>
              </ul>
              <Link className="btn btn-secondary" to={`/s/${slug}`}>
                回到服務列表
              </Link>
            </div>
          )}
        </div>

        <div className="flow-side">
          <SummaryCard
            service={service}
            staffLabel={staffLabel}
            start={step === 'sent' ? (sent?.booking.starts_at ?? null) : (choice?.start ?? null)}
            timezone={timezone}
          />
        </div>
      </div>
    </Shell>
  )
}

const STEPS: { key: Step; label: string }[] = [
  { key: 'time', label: '日期時間' },
  { key: 'details', label: '聯絡資料' },
  { key: 'sent', label: '信箱確認' },
]

/** 讓顧客知道自己在第幾步、還剩幾步 */
function Stepper({ current }: { current: Step }) {
  const currentIndex = STEPS.findIndex((s) => s.key === current)
  return (
    <ol className="stepper" aria-label="預約步驟">
      {STEPS.map((step, i) => {
        const state = i < currentIndex ? 'done' : i === currentIndex ? 'current' : 'todo'
        return (
          <li
            key={step.key}
            className={`step step-${state}`}
            aria-current={state === 'current' ? 'step' : undefined}
          >
            <span className="step-dot">
              {state === 'done' ? <CheckIcon width={14} height={14} /> : i + 1}
            </span>
            <span className="step-label">{step.label}</span>
          </li>
        )
      })}
    </ol>
  )
}
