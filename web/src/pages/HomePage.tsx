import { Shell } from '../components/Shell'

export function HomePage() {
  return (
    <Shell title="線上預約">
      <div className="state">
        <h1>線上預約</h1>
        <p>請使用店家提供的預約連結。</p>
        <p className="muted small">連結格式:/s/店家代稱</p>
      </div>
    </Shell>
  )
}

export function NotFoundPage() {
  return (
    <Shell title="找不到頁面">
      <div className="state">
        <h1>找不到這個頁面</h1>
        <p>網址可能輸入錯誤,請回到店家提供的預約連結。</p>
      </div>
    </Shell>
  )
}
