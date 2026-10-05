import type { Member, Role } from './types'

/** 與後端 Role::can_manage 一致:owner 管 manager 與 staff,manager 只管 staff;自己可以退出(owner 除外)。
 *  這裡只決定「要不要顯示按鈕」,真正的檢查一律在後端。 */
export function canRemove(me: { id: string; role: Role }, target: Member): boolean {
  if (target.role === 'owner') return false
  if (target.user_id === me.id) return true
  if (me.role === 'owner') return true
  return me.role === 'manager' && target.role === 'staff'
}

/** 停用(離職):同樣的階級規則,而且對象要是啟用中的。成員可以停用自己(退出) */
export function canDeactivate(me: { id: string; role: Role }, target: Member): boolean {
  return target.active && canRemove(me, target)
}

/** 重新啟用:只有管理者以上(停用中的人自己已經進不了這家店) */
export function canReactivate(me: { id: string; role: Role }, target: Member): boolean {
  return !target.active && target.user_id !== me.id && canRemove(me, target)
}

/** 可以改某位成員的營業時間 / 休假:管理者以上,或本人 */
export function canEditSchedule(me: { id: string; role: Role }, targetUserId: string): boolean {
  return me.role === 'owner' || me.role === 'manager' || me.id === targetUserId
}
