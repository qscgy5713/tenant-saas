import type { Staff } from '../api/types'

/** 單選:null 代表「不指定」。用原生 radio,鍵盤與螢幕閱讀器自然可用 */
export function StaffPicker({
  staff,
  value,
  onChange,
}: {
  staff: Staff[]
  value: string | null
  onChange: (id: string | null) => void
}) {
  const options: { id: string | null; label: string }[] = [
    { id: null, label: '不指定' },
    ...staff.map((s) => ({ id: s.id, label: s.name })),
  ]
  return (
    <fieldset className="staff-picker">
      <legend>選擇服務人員</legend>
      <div className="pills">
        {options.map((o) => (
          <label key={o.id ?? 'any'} className={`pill ${value === o.id ? 'pill-on' : ''}`}>
            <input
              type="radio"
              name="staff"
              checked={value === o.id}
              onChange={() => onChange(o.id)}
            />
            <span>{o.label}</span>
          </label>
        ))}
      </div>
    </fieldset>
  )
}
