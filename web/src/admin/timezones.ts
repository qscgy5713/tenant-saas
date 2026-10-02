/** 常見的店家時區。後端接受任何 IANA 時區,這裡只列常用的 */
export const TIMEZONES: { value: string; label: string }[] = [
  { value: 'Asia/Taipei', label: '台北 (GMT+8)' },
  { value: 'Asia/Hong_Kong', label: '香港 (GMT+8)' },
  { value: 'Asia/Shanghai', label: '上海 (GMT+8)' },
  { value: 'Asia/Singapore', label: '新加坡 (GMT+8)' },
  { value: 'Asia/Tokyo', label: '東京 (GMT+9)' },
  { value: 'Asia/Seoul', label: '首爾 (GMT+9)' },
  { value: 'Australia/Sydney', label: '雪梨' },
  { value: 'Pacific/Auckland', label: '奧克蘭' },
  { value: 'Europe/London', label: '倫敦' },
  { value: 'America/New_York', label: '紐約' },
  { value: 'America/Los_Angeles', label: '洛杉磯' },
]
