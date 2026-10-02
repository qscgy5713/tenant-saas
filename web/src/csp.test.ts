import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

/**
 * 正式環境的 nginx 設了 `style-src 'self'; script-src 'self'`(見 nginx.conf.template),
 * 行內 style 屬性與行內 script 會被瀏覽器直接擋掉。開發環境(Vite)沒有這個 CSP,
 * 所以「開發時看起來正常、上線後壞掉」。這個測試在建置前就把它擋下來。
 */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) return name === 'test' ? [] : sourceFiles(path)
    return /\.(tsx|ts|html)$/.test(name) && !/\.test\./.test(name) ? [path] : []
  })
}

const here = dirname(fileURLToPath(import.meta.url))
const files = [...sourceFiles(here), join(here, '..', 'index.html')]

describe('與 nginx 的 CSP 相容', () => {
  it('找得到要檢查的檔案(避免路徑錯了讓下面的檢查空轉)', () => {
    expect(files.length).toBeGreaterThan(30)
  })

  it('沒有 JSX 行內 style={{…}}', () => {
    const offenders = files.filter((f) => /style=\{\{/.test(readFileSync(f, 'utf8')))
    expect(offenders).toEqual([])
  })

  it('沒有 HTML 行內 style="…" 或行內 <script>', () => {
    const offenders = files.filter((f) => {
      const text = readFileSync(f, 'utf8')
      return /\sstyle="/.test(text) || /<script(?![^>]*\ssrc=)[^>]*>/.test(text)
    })
    expect(offenders).toEqual([])
  })

  it('沒有 eval / new Function(script-src 不含 unsafe-eval)', () => {
    const offenders = files.filter((f) => /\beval\(|new Function\(/.test(readFileSync(f, 'utf8')))
    expect(offenders).toEqual([])
  })

  it('沒有 dangerouslySetInnerHTML(顧客與店家輸入的內容都不可信)', () => {
    const offenders = files.filter((f) => /dangerouslySetInnerHTML/.test(readFileSync(f, 'utf8')))
    expect(offenders).toEqual([])
  })
})
