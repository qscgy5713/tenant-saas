import { formatDuration, formatPrice } from '../lib/money'
import { formatDateTime } from '../lib/time'
import type { Service } from '../api/types'

export function SummaryCard({
  service,
  staffLabel,
  start,
  timezone,
}: {
  service: Service
  staffLabel: string
  start: string | null
  timezone: string
}) {
  return (
    <aside className="summary" aria-label="預約摘要">
      <h2>預約摘要</h2>
      <dl>
        <div>
          <dt>服務</dt>
          <dd>
            <strong>{service.name}</strong>
            <span className="muted">
              {formatDuration(service.duration_minutes)} · {formatPrice(service.price_cents)}
            </span>
          </dd>
        </div>
        <div>
          <dt>服務人員</dt>
          <dd>{staffLabel}</dd>
        </div>
        <div>
          <dt>時間</dt>
          <dd>
            {start ? formatDateTime(start, timezone) : <span className="muted">尚未選擇</span>}
          </dd>
        </div>
      </dl>
    </aside>
  )
}
