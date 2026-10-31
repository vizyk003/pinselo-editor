import { BadgeCheck, CheckSquare, AtSign, Bell, CalendarClock, MessageSquare, Reply, Share2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router';
import { notifications, type Notification } from '../api';
import { PopoverButton, timeAgo } from './ui';
import { t } from '../i18n';

const ICON: Partial<Record<Notification['kind'], typeof Bell>> = { mention: AtSign, comment: MessageSquare, share: Share2, reply: Reply, assign: CheckSquare, publish: CalendarClock, approval: BadgeCheck };

/** In-app notifications (CO-070): polled every 30 s. */
export function NotificationsBell({ variant = 'side' }: { variant?: 'side' | 'header' }) {
  const nav = useNavigate();
  const [data, setData] = useState<{ items: Notification[]; unread: number }>({ items: [], unread: 0 });
  const load = useCallback(() => notifications.list().then(setData).catch(() => {}), []);
  useEffect(() => {
    void load();
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  }, [load]);
  return (
    <PopoverButton
      className={`btn ${variant === 'header' ? 'ghost sm' : 'ghost'} icon bell`}
      title={t('Notifications')}
      placement="below"
      label={<><Bell size={variant === 'header' ? 16 : 18} />{data.unread > 0 && <span className="dot">{data.unread}</span>}</>}
      content={(close) => (
        <div className="pop-md" style={{ display: 'grid', gap: 4 }}>
          <div className="row" style={{ padding: '4px 6px' }}>
            <b className="grow">{t('Notifications')}</b>
            {data.unread > 0 && <button className="btn ghost sm" onClick={async () => { await notifications.read(); void load(); }}>{t('Mark all read')}</button>}
          </div>
          <div className="scroll" style={{ maxHeight: 420 }}>
            {!data.items.length && <div className="empty">{t('You’re all caught up.')}</div>}
            {data.items.map((n) => {
              const Icon = ICON[n.kind] ?? Bell;
              return (
                <button key={n.id} className={`notif ${n.read_at ? '' : 'unread'}`} onClick={async () => {
                  close();
                  await notifications.read([n.id]);
                  void load();
                  if (n.kind === 'publish') nav('/planner');
                  else if (n.design_id) nav(`/design/${n.design_id}`);
                }}>
                  <span className="avatar" style={{ background: n.actor_color ?? '#64748b', width: 28, height: 28 }}><Icon size={14} /></span>
                  <span>
                    <span style={{ display: 'block', fontSize: 13 }}>{n.body}</span>
                    <span className="faint" style={{ fontSize: 11 }}>{n.design_title ? `${n.design_title} · ` : ''}{timeAgo(n.created_at)}</span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    />
  );
}
