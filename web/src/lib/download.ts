/** 把檔案存到使用者的電腦(瀏覽器的下載)。獨立成一個函式,測試才能替換掉(jsdom 沒有 createObjectURL) */
export function saveFile(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  document.body.append(link)
  link.click()
  link.remove()
  // 下載是非同步的,太快釋放有些瀏覽器會失敗
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}
