import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { ErrorState, Loading } from '../../components/States'
import { AUDIT_FILTERS, actionLabel, actorLabel, describeDetail } from '../../lib/audit'
import { formatDateTime } from '../../lib/time'
import { exportAudit, listAudit } from '../api'
import { ExportDialog } from '../components/ExportDialog'
import { useShop } from '../ShopContext'

const PAGE_SIZE = 30

/** 稽核日誌(只有擁有者與管理者看得到):誰在什麼時候改了什麼。唯讀,資料庫層面也不能修改或刪除。 */
export function AuditPage() {
  const { slug, shop } = useShop()
  const [filter, setFilter] = useState('')
  const [exporting, setExporting] = useState(false)
  const queryClient = useQueryClient()
  const filterLabel = AUDIT_FILTERS.find((f) => f.value === filter)?.label ?? '全部'

  const query = useInfiniteQuery({
    queryKey: ['admin', 'audit', slug, filter],
    queryFn: ({ pageParam }) =>
      listAudit(slug, { action: filter || undefined, before: pageParam, limit: PAGE_SIZE }),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
    staleTime: 10_000,
  })

  const entries = query.data?.pages.flatMap((p) => p.items) ?? []

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">稽核日誌</h1>
          <p className="muted small">
            記錄誰在什麼時候做了什麼。日誌只能新增、不能修改或刪除,而且不會記錄
            Email、電話、備註內容或休假原因。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-secondary" onClick={() => setExporting(true)}>
            匯出 CSV
          </button>
        </div>
      </div>

      <div className="toolbar">
        <div className="segmented" role="group" aria-label="類別">
          {AUDIT_FILTERS.map((f) => (
            <button
              key={f.value}
              type="button"
              aria-pressed={filter === f.value}
              onClick={() => setFilter(f.value)}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {query.isPending && <Loading label="載入日誌…" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => query.refetch()} />}
      {query.data && entries.length === 0 && (
        <div className="empty-card">
          <p>沒有符合的紀錄。</p>
        </div>
      )}

      <ul className="rows">
        {entries.map((e) => {
          const detail = describeDetail(e, shop.timezone)
          return (
            <li key={e.id} className="row audit-row">
              <div className="time-col small">{formatDateTime(e.created_at, shop.timezone)}</div>
              <div className="row-main">
                <div className="row-title">{actionLabel(e.action)}</div>
                {detail && <div className="row-sub">{detail}</div>}
              </div>
              <div className="row-sub audit-actor">{actorLabel(e)}</div>
            </li>
          )
        })}
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

      <ExportDialog
        open={exporting}
        onClose={() => setExporting(false)}
        title="匯出稽核日誌"
        description={
          <>
            匯出成 CSV(可用 Excel 開啟),類別:<strong>{filterLabel}</strong>。一次最多 50,000
            筆,超過請縮小日期範圍。匯出本身也會被記錄。
          </>
        }
        successNote="已匯出,檔案正在下載。這次匯出也已記錄在稽核日誌裡。"
        run={(range) => exportAudit(slug, { action: filter || undefined, ...range })}
        // 匯出本身會留下一筆稽核紀錄,讓列表重新整理就看得到
        onExported={() => queryClient.invalidateQueries({ queryKey: ['admin', 'audit', slug] })}
      />
    </div>
  )
}
