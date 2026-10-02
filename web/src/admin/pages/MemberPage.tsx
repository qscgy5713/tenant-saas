import { Link, useParams } from 'react-router-dom'
import { Avatar } from '../../components/Avatar'
import { ChevronLeftIcon } from '../../components/Icons'
import { ErrorState, Loading } from '../../components/States'
import { StaffServicesSection } from '../components/StaffServicesSection'
import { TimeOffSection } from '../components/TimeOffSection'
import { WorkingHoursEditor } from '../components/WorkingHoursEditor'
import { canEditSchedule } from '../permissions'
import { useMembers } from '../queries'
import { ROLE_LABEL, useShop } from '../ShopContext'

/** 一位成員的營業時間、可提供的服務與休假 */
export function MemberPage() {
  const { userId = '' } = useParams()
  const { slug, shop, user, role, canManage } = useShop()
  const members = useMembers(slug)

  if (members.isPending) return <Loading />
  if (members.isError) return <ErrorState error={members.error} onRetry={() => members.refetch()} />

  const member = members.data.find((m) => m.user_id === userId)
  if (!member) {
    return (
      <div className="state">
        <h1>找不到這位成員</h1>
        <Link className="btn btn-secondary" to={`/admin/${slug}/team`}>
          回到團隊
        </Link>
      </div>
    )
  }

  const editSchedule = canEditSchedule({ id: user.id, role }, member.user_id)

  return (
    <div>
      <Link className="back-link" to={`/admin/${slug}/team`}>
        <ChevronLeftIcon width={16} height={16} />
        團隊
      </Link>
      <div className="page-head">
        <div className="member-id">
          <Avatar name={member.name} size="lg" />
          <div>
            <h1 className="page-heading">{member.name}</h1>
            <p className="muted small">
              {member.email} · {ROLE_LABEL[member.role]}
            </p>
          </div>
        </div>
      </div>
      {!editSchedule && <p className="muted">你可以查看,但只有管理者或本人能修改。</p>}

      <section className="section card-section">
        <h2>營業時間</h2>
        <p className="muted small">每週固定的上班時段(店家當地時間)。顧客只能預約這些時段。</p>
        <WorkingHoursEditor slug={slug} userId={member.user_id} editable={editSchedule} />
      </section>

      <section className="section card-section">
        <h2>可提供的服務</h2>
        <StaffServicesSection slug={slug} userId={member.user_id} editable={canManage} />
      </section>

      <section className="section card-section">
        <h2>休假</h2>
        <TimeOffSection
          slug={slug}
          userId={member.user_id}
          timezone={shop.timezone}
          editable={editSchedule}
        />
      </section>
    </div>
  )
}
