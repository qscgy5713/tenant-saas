// 沒有照片時,用名字的第一個字做頭像。顏色由名字決定(同一個名字永遠同一個顏色)。
// 顏色與尺寸都用 class,不用行內 style:正式環境的 CSP 是 style-src 'self'
const TONE_COUNT = 6

export type AvatarSize = 'sm' | 'md' | 'lg'

function toneOf(name: string): number {
  let hash = 0
  for (const ch of name) hash = (hash * 31 + (ch.codePointAt(0) ?? 0)) >>> 0
  return hash % TONE_COUNT
}

export function Avatar({ name, size = 'md' }: { name: string; size?: AvatarSize }) {
  const initial = [...name.trim()][0] ?? '?'
  return (
    <span className={`avatar avatar-${size} avatar-tone-${toneOf(name)}`} aria-hidden="true">
      {initial}
    </span>
  )
}
