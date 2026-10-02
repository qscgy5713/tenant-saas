import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { ErrorState, Loading } from '../../components/States'
import { formatDateTime, formatDayLong, ymdInTz } from '../../lib/time'
import { getCustomer, listCustomers } from '../api'
import { Modal } from '../components/Modal'
import { STATUS_LABEL } from '../labels'
import { useShop } from '../ShopContext'
import type { Customer } from '../types'

const PAGE_SIZE = 30

/** 搜尋框停止輸入一小段時間後才送出,不用每個字都查一次 */
function useDebounced<T>(value: T, ms = 300): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), ms)
    return () => clearTimeout(t)
  }, [value, ms])
  return debounced
}

/** 顧客清單(擁有者與管理者)。只列確認過的顧客:光是有人輸入某個 Email 提出申請,不算。 */
export function CustomersPage() {
  const { slug, shop } = useShop()
  const [search, setSearch] = useState('')
  const q = useDebounced(search.trim())
  const [openId, setOpenId] = useState<string | null>(null)

  const query = useInfiniteQuery({
    queryKey: ['admin', 'customers', slug, q],
    queryFn: ({ pageParam }) =>
      listCustomers(slug, { q: q || undefined, limit: PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.items.length >= PAGE_SIZE ? last.offset + PAGE_SIZE : undefined,
    staleTime: 10_000,
  })
  const customers = query.data?.pages.flatMap((p) => p.items) ?? []
  const tz = shop.timezone

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">顧客</h1>
          <p className="muted small">有過已確認預約的顧客,依最近一次預約排序。</p>
        </div>
      </div>

      <div className="toolbar">
        <input
          type="search"
          aria-label="搜尋顧客"
          placeholder="姓名、Email 或電話"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          maxLength={100}
        />
      </div>

      {query.isPending && <Loading label="載入顧客…" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => query.refetch()} />}
      {query.data && customers.length === 0 && (
        <div className="empty-card">
          <p>{q ? '沒有符合的顧客。' : '還沒有顧客。有人完成預約後會出現在這裡。'}</p>
        </div>
      )}

      <ul className="rows">
        {customers.map((c) => (
          <CustomerRow key={c.id} customer={c} tz={tz} onOpen={() => setOpenId(c.id)} />
        ))}
      </ul>

      {query.hasNextPage && (
        <div className="load-more">
          <button
            type="button"
            className="btn btn-secondary"
            disabled={query.isFetchingNextPage}
            onClick={() => query.fetchNextPage()}
          >
            {query.isFetchingNextPage ? '載入中…' : '載入更多'}
          </button>
        </div>
      )}

      <Modal open={!!openId} title="顧客資料" onClose={() => setOpenId(null)} size="lg">
        {openId && <CustomerDetailView id={openId} />}
      </Modal>
    </div>
  )
}

function CustomerRow({
  customer: c,
  tz,
  onOpen,
}: {
  customer: Customer
  tz: string
  onOpen: () => void
}) {
  return (
    <li className="row">
      <div className="row-main">
        <div className="row-title">
          <button type="button" className="link-btn" onClick={onOpen}>
            {c.name}
          </button>
          {c.no_shows > 0 && (
            <span className="badge badge-no_show badge-inline">未到 {c.no_shows}</span>
          )}
        </div>
        <div className="row-sub contact">
          <a href={`mailto:${c.email}`}>{c.email}</a>
          {c.phone && <a href={`tel:${c.phone}`}>{c.phone}</a>}
        </div>
      </div>
      <div className="row-sub customer-stats">
        <div>共 {c.bookings} 次預約</div>
        {c.next_at ? (
          <div>下次:{formatDayLong(ymdInTz(c.next_at, tz))}</div>
        ) : c.last_at ? (
          <div>最近:{formatDayLong(ymdInTz(c.last_at, tz))}</div>
        ) : null}
      </div>
    </li>
  )
}

function CustomerDetailView({ id }: { id: string }) {
  const { slug, shop } = useShop()
  const detail = useQuery({
    queryKey: ['admin', 'customers', slug, 'detail', id],
    queryFn: () => getCustomer(slug, id),
    staleTime: 10_000,
  })
  if (detail.isPending) return <Loading />
  if (detail.isError) return <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
  const c = detail.data
  return (
    <div>
      <p>
        <strong>{c.name}</strong>
        <br />
        <a href={`mailto:${c.email}`}>{c.email}</a>
        {c.phone && (
          <>
            {' · '}
            <a href={`tel:${c.phone}`}>{c.phone}</a>
          </>
        )}
      </p>
      <p className="muted small">
        共 {c.bookings} 次預約,完成 {c.completed} 次,未到 {c.no_shows} 次
      </p>
      <h3>預約紀錄</h3>
      <ul className="rows">
        {c.history.map((h) => (
          <li key={h.id} className={`row booking-row booking-${h.status}`}>
            <div className="time-col">{formatDateTime(h.starts_at, shop.timezone)}</div>
            <div className="row-main">
              <div className="row-title">
                {h.service_name}
                <span className={`badge badge-${h.status} badge-inline`}>
                  {STATUS_LABEL[h.status]}
                </span>
              </div>
              <div className="row-sub">{h.staff_name}</div>
              {h.notes && <div className="row-note">備註:{h.notes}</div>}
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}
