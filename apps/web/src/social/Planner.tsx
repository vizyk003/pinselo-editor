/** Content Planner (A16): month/week calendar with drag-to-reschedule, list view, account filter. */
import { CalendarDays, CalendarRange, Check, ChevronLeft, ChevronRight, ExternalLink, List, MessageSquareWarning, Plus, Send, Pencil, Trash2 } from 'lucide-react';
import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { api, ApiError, thumbUrl, type DesignSummary } from '../api';
import { EmptyState, Modal, PageHeader, useToast } from '../components/ui';
import { fmtDate, getLocale, t } from '../i18n';
import { CHANNEL_LABEL, EDITORIAL_FIELDS, isEditable, PROVIDER_COLOR, socialApi, STATUS_LABEL, type PostEvent, type SocialAccount, type SocialPost } from './api';
import { AccountBadge } from './AccountBadge';
import { ScheduleDialog } from './ScheduleDialog';
import './planner.css';

type View = 'month' | 'week' | 'list';
const DAY = 86_400_000;
const VIEW_KEY = 'oc.planner.view';

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
/** Weeks start on Monday. */
const startOfWeek = (d: Date) => addDays(startOfDay(d), -((d.getDay() + 6) % 7));
const timeFmt = (iso: string) => new Date(iso).toLocaleTimeString(getLocale(), { hour: 'numeric', minute: '2-digit' });
const editable = isEditable;
/** "P-001 · " before the design title, unless the title already starts with the post ID. */
const refPrefix = (p: SocialPost) => (p.fields.ref && !(p.designTitle ?? '').startsWith(p.fields.ref) ? `${p.fields.ref} · ` : '');
/** Statuses the review filter offers. */
const REVIEW_FILTERS = ['in_review', 'changes_requested', 'approved', 'failed'] as const;

export function PlannerPage() {
  const toast = useToast();
  const [view, setView] = useState<View>(() => {
    try { return (localStorage.getItem(VIEW_KEY) as View) || 'month'; } catch { return 'month'; }
  });
  const [cursor, setCursor] = useState(() => startOfDay(new Date()));
  const [accounts, setAccounts] = useState<SocialAccount[]>([]);
  const [filter, setFilter] = useState('');
  const [statusFilter, setStatusFilter] = useState('');
  const [posts, setPosts] = useState<SocialPost[] | null>(null);
  const [open, setOpen] = useState<SocialPost | null>(null);
  const [creating, setCreating] = useState<Date | null>(null);
  const [dropDay, setDropDay] = useState<number | null>(null);

  useEffect(() => {
    try { localStorage.setItem(VIEW_KEY, view); } catch { /* storage blocked */ }
  }, [view]);

  const range = useMemo(() => {
    if (view === 'week') {
      const s = startOfWeek(cursor);
      return { from: s, to: addDays(s, 7), days: [...Array(7).keys()].map((i) => addDays(s, i)) };
    }
    if (view === 'month') {
      const first = new Date(cursor.getFullYear(), cursor.getMonth(), 1);
      const s = startOfWeek(first);
      const last = new Date(cursor.getFullYear(), cursor.getMonth() + 1, 0);
      const weeks = Math.ceil((Math.round((startOfDay(last).getTime() - s.getTime()) / DAY) + 1) / 7);
      return { from: s, to: addDays(s, weeks * 7), days: [...Array(weeks * 7).keys()].map((i) => addDays(s, i)) };
    }
    return { from: null, to: null, days: [] };
  }, [view, cursor]);

  const load = useCallback(async () => {
    const r = await socialApi.posts({ from: range.from?.toISOString(), to: range.to?.toISOString(), account: filter || undefined, status: statusFilter || undefined });
    setPosts(r.posts);
  }, [range.from, range.to, filter, statusFilter]);

  useEffect(() => {
    void load().catch(() => setPosts([]));
  }, [load]);
  useEffect(() => {
    void socialApi.accounts().then((r) => setAccounts(r.accounts)).catch(() => {});
    // Publishing finishes in the background: refresh while the page is open.
    const timer = setInterval(() => void load().catch(() => {}), 30_000);
    return () => clearInterval(timer);
  }, [load]);

  const step = (dir: number) => {
    if (view === 'week') setCursor(addDays(cursor, 7 * dir));
    else setCursor(new Date(cursor.getFullYear(), cursor.getMonth() + dir, 1));
  };
  const period = view === 'week'
    ? `${fmtDate(range.from!, { month: 'short', day: 'numeric' })} – ${fmtDate(addDays(range.to!, -1), { month: 'short', day: 'numeric', year: 'numeric' })}`
    : cursor.toLocaleDateString(getLocale(), { month: 'long', year: 'numeric' });

  const reschedule = async (p: SocialPost, day: Date) => {
    const old = p.scheduledAt ? new Date(p.scheduledAt) : new Date(day.getFullYear(), day.getMonth(), day.getDate(), 9);
    const next = new Date(day.getFullYear(), day.getMonth(), day.getDate(), old.getHours(), old.getMinutes());
    if (p.scheduledAt && sameDay(old, next)) return;
    if (p.status === 'scheduled' && next.getTime() < Date.now()) return toast(t('Pick a time in the future.'));
    if (p.approvedAt && !confirm(t('This post is approved. Moving it sends it back to review. Move it?'))) return;
    setPosts((list) => list?.map((x) => (x.id === p.id ? { ...x, scheduledAt: next.toISOString() } : x)) ?? null);
    try {
      // Review posts keep their place in the workflow (the server sends approved ones back to review).
      const keep = p.publisher === 'external' || ['in_review', 'changes_requested', 'approved'].includes(p.status) || p.approvedAt;
      await socialApi.update(p.id, keep ? { scheduledAt: next.toISOString() } : { scheduledAt: next.toISOString(), status: p.status === 'draft' ? 'draft' : 'scheduled' });
      toast(t('Moved to {date}', { date: fmtDate(next, { weekday: 'short', month: 'short', day: 'numeric' }) }));
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
    }
    void load();
  };

  const visible = posts ?? [];
  const dayPosts = (d: Date) => visible.filter((p) => p.scheduledAt && sameDay(new Date(p.scheduledAt), d)).sort((a, b) => a.scheduledAt!.localeCompare(b.scheduledAt!));
  const today = new Date();
  const dows = [...Array(7).keys()].map((i) => addDays(startOfWeek(today), i).toLocaleDateString(getLocale(), { weekday: 'short' }));

  return (
    <>
    <PageHeader title={t('Content Planner')} className="planner-head">
        <div className="grow" />
        <div className="segmented" role="tablist" aria-label={t('View')}>
          <button role="tab" aria-selected={view === 'month'} className={view === 'month' ? 'on' : ''} onClick={() => setView('month')}><CalendarDays size={15} /> {t('Month')}</button>
          <button role="tab" aria-selected={view === 'week'} className={view === 'week' ? 'on' : ''} onClick={() => setView('week')}><CalendarRange size={15} /> {t('Week')}</button>
          <button role="tab" aria-selected={view === 'list'} className={view === 'list' ? 'on' : ''} onClick={() => setView('list')}><List size={15} /> {t('List')}</button>
        </div>
        <button className="btn primary" onClick={() => setCreating(new Date())}><Plus size={16} /> {t('Schedule a post')}</button>
    </PageHeader>
    <div className="content planner">
      <div className="planner-bar">
        {view !== 'list' && (
          <>
            <button className="btn" onClick={() => setCursor(startOfDay(new Date()))}>{t('Today')}</button>
            <button className="btn ghost icon" aria-label={t('Previous')} onClick={() => step(-1)}><ChevronLeft size={18} /></button>
            <button className="btn ghost icon" aria-label={t('Next')} onClick={() => step(1)}><ChevronRight size={18} /></button>
            <span className="period" aria-live="polite">{period}</span>
          </>
        )}
        <div className="grow" />
        <select className="input" style={{ width: 'auto', minWidth: 170 }} value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} aria-label={t('Filter by status')}>
          <option value="">{t('All statuses')}</option>
          {REVIEW_FILTERS.map((st) => <option key={st} value={st}>{st === 'in_review' ? t('Needs approval') : t(STATUS_LABEL[st])}</option>)}
        </select>
        <select className="input" style={{ width: 'auto', minWidth: 200 }} value={filter} onChange={(e) => setFilter(e.target.value)} aria-label={t('Filter by account')}>
          <option value="">{t('All accounts')}</option>
          {accounts.map((a) => <option key={a.id} value={a.id}>{a.provider === 'local' ? t('Download & remind') : `${a.displayName}${a.handle ? ` (@${a.handle})` : ''}`}</option>)}
        </select>
        <Link className="btn ghost" to="/settings/accounts">{t('Connected accounts')}</Link>
      </div>

      {posts === null ? <div className="spinner" /> : view === 'list' ? (
        <PostList posts={visible} accounts={accounts} onOpen={setOpen} />
      ) : (
        <div className={`cal ${view}`} role="grid" aria-label={period}>
          {dows.map((d) => <div key={d} className="dow" role="columnheader">{d}</div>)}
          {range.days.map((d) => {
            const key = d.getTime();
            const other = view === 'month' && d.getMonth() !== cursor.getMonth();
            return (
              <div key={key} role="gridcell" className={`day ${other ? 'other' : ''} ${sameDay(d, today) ? 'today' : ''} ${dropDay === key ? 'drop' : ''}`}
                onDragOver={(e) => {
                  if (!e.dataTransfer.types.includes('application/x-oc-post')) return;
                  e.preventDefault();
                  setDropDay(key);
                }}
                onDragLeave={() => setDropDay((k) => (k === key ? null : k))}
                onDrop={(e) => {
                  e.preventDefault();
                  setDropDay(null);
                  const p = visible.find((x) => x.id === e.dataTransfer.getData('application/x-oc-post'));
                  if (p) void reschedule(p, d);
                }}>
                <div className="head">
                  <span className="num">{d.getDate()}</span>
                  {d.getTime() + DAY > Date.now() && (
                    <button className="add" title={t('Schedule a post')} aria-label={t('Schedule a post on {date}', { date: fmtDate(d, { month: 'long', day: 'numeric' }) })} onClick={() => setCreating(d)}><Plus size={14} /></button>
                  )}
                </div>
                {dayPosts(d).map((p) => <PostPill key={p.id} post={p} accounts={accounts} onOpen={() => setOpen(p)} />)}
              </div>
            );
          })}
        </div>
      )}
      {posts !== null && !visible.length && view === 'list' && (
        <EmptyState icon={<CalendarDays size={22} />} action={<button className="btn primary" onClick={() => setCreating(new Date())}><Plus size={16} /> {t('Schedule a post')}</button>}>
          {t('Nothing scheduled yet. Open a design and choose Schedule, or start here.')}
        </EmptyState>
      )}

      {open && <PostDetails post={open} accounts={accounts} onClose={() => setOpen(null)} onChanged={() => { setOpen(null); void load(); }} />}
      {creating && <PickDesign at={creating} onClose={() => setCreating(null)} onSaved={() => void load()} />}
    </div>
    </>
  );
}

function Nets({ post, accounts, size = 16 }: { post: SocialPost; accounts: SocialAccount[]; size?: number }) {
  return (
    <span className="nets">
      {post.publisher === 'external' && post.channels.map((c) => (
        <span key={c} className="chan-badge" title={CHANNEL_LABEL[c]} style={{ background: PROVIDER_COLOR[c], minWidth: size, height: size }}>{c === 'instagram' ? 'IG' : 'f'}</span>
      ))}
      {post.accounts.slice(0, 4).map((id) => {
        const a = accounts.find((x) => x.id === id);
        return a ? <AccountBadge key={id} account={{ ...a, avatarUrl: null }} size={size} /> : null;
      })}
    </span>
  );
}

function PostPill({ post, accounts, onOpen }: { post: SocialPost; accounts: SocialAccount[]; onOpen: () => void }) {
  const can = editable(post);
  return (
    <button className={`post-pill ${post.status} ${can ? '' : 'locked'}`} draggable={can} onClick={onOpen}
      onDragStart={(e) => {
        e.dataTransfer.setData('application/x-oc-post', post.id);
        e.dataTransfer.effectAllowed = 'move';
      }}
      title={`${post.designTitle ?? ''} — ${t(STATUS_LABEL[post.status])}`}>
      <img className="thumb" src={thumbUrl(post.designId)} alt="" loading="lazy" />
      <span className="txt">
        <b>{post.scheduledAt ? timeFmt(post.scheduledAt) : t('No time')}</b>
        <span>{refPrefix(post)}{post.designTitle}</span>
      </span>
      <Nets post={post} accounts={accounts} />
    </button>
  );
}

function PostList({ posts, accounts, onOpen }: { posts: SocialPost[]; accounts: SocialAccount[]; onOpen: (p: SocialPost) => void }) {
  const now = Date.now();
  const upcoming = posts.filter((p) => !p.scheduledAt || new Date(p.scheduledAt).getTime() >= now || ['scheduled', 'publishing', 'in_review', 'changes_requested', 'approved'].includes(p.status));
  const past = posts.filter((p) => !upcoming.includes(p)).reverse();
  const section = (title: string, list: SocialPost[]) => list.length > 0 && (
    <section className="col">
      <h3 style={{ margin: '8px 0 0' }}>{title}</h3>
      <div className="post-list">
        {list.map((p) => (
          <div key={p.id} className="post-row">
            <img src={thumbUrl(p.designId)} alt="" loading="lazy" />
            <div className="meta">
              <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
                <b>{refPrefix(p)}{p.designTitle}</b>
                <span className={`status-dot ${p.status}`}>{t(STATUS_LABEL[p.status])}</span>
                <span className="faint" style={{ fontSize: 12 }}>
                  {p.scheduledAt ? new Date(p.scheduledAt).toLocaleString(getLocale(), { dateStyle: 'medium', timeStyle: 'short' }) : t('No time')}
                </span>
                <Nets post={p} accounts={accounts} size={18} />
              </div>
              <span className="cap">{p.caption || <i className="faint">{t('No caption')}</i>}</span>
              {p.status === 'failed' && p.error && <span className="error" style={{ fontSize: 12 }}>{p.error}</span>}
            </div>
            <button className="btn sm" onClick={() => onOpen(p)}>{t('Details')}</button>
          </div>
        ))}
      </div>
    </section>
  );
  return <>{section(t('Upcoming'), upcoming)}{section(t('Past'), past)}</>;
}

function PostDetails({ post, accounts, onClose, onChanged }: { post: SocialPost; accounts: SocialAccount[]; onClose: () => void; onChanged: () => void }) {
  const toast = useToast();
  const nav = useNavigate();
  const [events, setEvents] = useState<PostEvent[]>([]);
  const [design, setDesign] = useState<DesignSummary | null>(null);
  const [editing, setEditing] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const act = async (fn: () => Promise<unknown>, done: string) => {
    setBusy(true);
    try {
      await fn();
      toast(done);
      onChanged();
    } catch (e) {
      toast(e instanceof ApiError ? (e.message || e.code) : t('That didn’t work. Try again.'));
    } finally {
      setBusy(false);
    }
  };
  const external = post.publisher === 'external';
  const shownFields = EDITORIAL_FIELDS.filter((f) => post.fields[f.key]);
  useEffect(() => {
    void socialApi.events(post.id).then((r) => setEvents(r.events)).catch(() => {});
    void api.design(post.designId).then((r) => setDesign(r.design)).catch(() => {});
  }, [post.id, post.designId]);
  const name = (id: string | null) => {
    const a = accounts.find((x) => x.id === id);
    return a ? (a.provider === 'local' ? t('Download & remind') : a.displayName) : '';
  };
  if (editing) {
    return <ScheduleDialog designId={post.designId} title={post.designTitle ?? ''} pageCount={design?.page_count ?? Math.max(1, ...post.pages.map((p) => p + 1))} post={post} onClose={() => setEditing(false)} onSaved={onChanged} />;
  }
  return (
    <Modal title={`${refPrefix(post)}${post.designTitle ?? t('Scheduled post')}`} onClose={onClose} size="lg">
      <div className="planner-form">
        <div className="row" style={{ gap: 14, alignItems: 'flex-start' }}>
          <img src={thumbUrl(post.designId)} alt="" style={{ width: 120, borderRadius: 10, background: 'var(--surface-3)' }} />
          <div className="col grow" style={{ gap: 6 }}>
            <span className={`status-dot ${post.status}`}>{t(STATUS_LABEL[post.status])}</span>
            <span>{post.scheduledAt ? new Date(post.scheduledAt).toLocaleString(getLocale(), { dateStyle: 'full', timeStyle: 'short', timeZone: post.timezone }) : t('No time')} <span className="faint">({post.timezone.replace(/_/g, ' ')})</span></span>
            <span className="faint" style={{ fontSize: 13 }}>{post.format === 'mp4' ? t('Video (MP4)') : `${post.format.toUpperCase()} · ${post.pages.length ? t('Pages {list}', { list: post.pages.map((p) => p + 1).join(', ') }) : t('All pages')}`}</span>
            {external && <span className="faint" style={{ fontSize: 13 }}>{t('Published by your own publisher')}</span>}
            {post.approvedAt && (
              <span style={{ fontSize: 13 }}>
                <Check size={13} /> {t('Approved by {name} on {date}', { name: post.approvedByName ?? t('a reviewer'), date: new Date(post.approvedAt).toLocaleString(getLocale(), { dateStyle: 'medium', timeStyle: 'short' }) })}
                {post.approvedVersion != null && <span className="faint"> · {t('design version {n}', { n: post.approvedVersion })}</span>}
              </span>
            )}
            {post.error && <span className="error" style={{ fontSize: 13 }}>{post.error}</span>}
          </div>
        </div>
        {shownFields.length > 0 && (
          <dl className="fields-table">
            {shownFields.map((f) => <Fragment key={f.key}><dt>{t(f.label)}</dt><dd>{post.fields[f.key]}</dd></Fragment>)}
          </dl>
        )}
        {(post.caption || post.hashtags.length > 0) && (
          <div className="box" style={{ padding: 12, whiteSpace: 'pre-wrap', fontSize: 14 }}>
            {post.caption}{post.hashtags.length > 0 && <div className="faint" style={{ marginTop: 6 }}>{post.hashtags.map((h) => `#${h}`).join(' ')}</div>}
          </div>
        )}
        {external && (
          <div className="col" style={{ gap: 6 }}>
            {post.channels.map((c) => {
              const r = post.results[c];
              return (
                <div key={c} className="account-row">
                  <span className="chan-badge" style={{ background: PROVIDER_COLOR[c], minWidth: 26, height: 26, fontSize: 11 }}>{c === 'instagram' ? 'IG' : 'f'}</span>
                  <div className="who"><b>{CHANNEL_LABEL[c]}</b>{r?.error && <span className="error" style={{ fontSize: 12 }}>{r.error}</span>}</div>
                  {r?.status === 'published' && r.url && <a className="btn sm" href={r.url} target="_blank" rel="noreferrer"><ExternalLink size={14} /> {t('View post')}</a>}
                  {r?.status === 'published' && !r.url && <span className="status-dot published">{t('Published')}</span>}
                  {r?.status === 'failed' && <span className="status-dot failed">{t('Failed')}</span>}
                  {r?.status === 'unknown' && <span className="status-dot publishing">{t('Check by hand')}</span>}
                </div>
              );
            })}
          </div>
        )}
        {note !== null && (
          <div className="review-box">
            <label className="field">
              <span>{t('What should change?')}</span>
              <textarea className="input" rows={3} value={note} autoFocus onChange={(e) => setNote(e.target.value)} />
            </label>
            <div className="row" style={{ justifyContent: 'flex-end', gap: 8 }}>
              <button className="btn ghost" onClick={() => setNote(null)}>{t('Cancel')}</button>
              <button className="btn primary" disabled={busy} onClick={() => act(() => socialApi.requestChanges(post.id, note), t('Changes requested'))}>{t('Request changes')}</button>
            </div>
          </div>
        )}
        <div className="col" style={{ gap: 6 }}>
          {post.accounts.map((id) => {
            const a = accounts.find((x) => x.id === id);
            const r = post.results[id];
            return (
              <div key={id} className="account-row">
                {a && <AccountBadge account={a} size={26} />}
                <div className="who"><b>{name(id) || t('Disconnected account')}</b>{r?.error && <span className="error" style={{ fontSize: 12 }}>{r.error}</span>}{r?.note && <span className="faint" style={{ fontSize: 12 }}>{r.note}</span>}</div>
                {r?.status === 'published' && r.url && <a className="btn sm" href={r.url} target="_blank" rel="noreferrer"><ExternalLink size={14} /> {a?.provider === 'local' ? t('Download') : t('View post')}</a>}
                {r && r.status !== 'published' && <span className={`status-dot ${r.status === 'failed' ? 'failed' : 'publishing'}`}>{r.status === 'failed' ? t('Failed') : t('Retrying')}</span>}
              </div>
            );
          })}
        </div>
        {events.length > 0 && (
          <details>
            <summary style={{ cursor: 'pointer', fontWeight: 600, fontSize: 13 }}>{t('Activity')}</summary>
            <ul className="event-log">
              {events.map((e) => (
                <li key={e.id}><time>{new Date(e.createdAt).toLocaleString(getLocale(), { dateStyle: 'short', timeStyle: 'medium' })}</time><span><b>{e.kind}</b>{e.accountId ? ` · ${name(e.accountId)}` : ''}{e.message ? ` — ${e.message}` : ''}</span></li>
              ))}
            </ul>
          </details>
        )}
        <div className="row" style={{ justifyContent: 'flex-end', gap: 8, flexWrap: 'wrap' }}>
          <button className="btn ghost" onClick={() => nav(`/design/${post.designId}`)}>{t('Open design')}</button>
          <div className="grow" />
          {post.status !== 'publishing' && (
            <button className="btn ghost danger" onClick={async () => {
              if (!confirm(t('Delete this scheduled post?'))) return;
              await socialApi.remove(post.id);
              onChanged();
            }}><Trash2 size={15} /> {t('Delete')}</button>
          )}
          {editable(post) && <button className="btn" onClick={() => setEditing(true)}><Pencil size={15} /> {t('Edit')}</button>}
          {['draft', 'changes_requested', 'failed'].includes(post.status) && (post.status !== 'failed' || external) && (
            <button className="btn" disabled={busy} onClick={() => act(() => socialApi.submit(post.id), t('Sent for review'))}><Send size={15} /> {t('Submit for review')}</button>
          )}
          {['in_review', 'approved', 'scheduled', 'failed'].includes(post.status) && note === null && (post.status !== 'scheduled' || post.approvedAt) && (
            <button className="btn" disabled={busy} onClick={() => setNote(post.fields.reviewerNote ?? '')}><MessageSquareWarning size={15} /> {t('Request changes')}</button>
          )}
          {(post.status === 'in_review' || (post.status === 'failed' && external)) && (
            <button className="btn primary" disabled={busy} onClick={() => act(() => socialApi.approve(post.id), t('Approved'))}><Check size={15} /> {t('Approve')}</button>
          )}
          {!external && ['draft', 'scheduled', 'failed'].includes(post.status) && post.accounts.length > 0 && (
            <button className="btn primary" onClick={async () => {
              try {
                await socialApi.publishNow(post.id);
                toast(t('Publishing now…'));
                onChanged();
              } catch (e) {
                toast(e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
              }
            }}><Send size={15} /> {post.status === 'failed' ? t('Retry now') : t('Publish now')}</button>
          )}
        </div>
      </div>
    </Modal>
  );
}

/** "Schedule a post" from the Planner: pick one of your designs first. */
function PickDesign({ at, onClose, onSaved }: { at: Date; onClose: () => void; onSaved: () => void }) {
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const [q, setQ] = useState('');
  const [picked, setPicked] = useState<DesignSummary | null>(null);
  useEffect(() => {
    void api.designs().then((r) => setDesigns(r.designs)).catch(() => setDesigns([]));
  }, []);
  if (picked) {
    return <ScheduleDialog designId={picked.id} title={picked.title} pageCount={picked.page_count || 1} video={picked.surface === 'video'} initialAt={at} onClose={onClose} onSaved={onSaved} />;
  }
  const list = (designs ?? []).filter((d) => !d.trashed_at && d.title.toLowerCase().includes(q.toLowerCase())).slice(0, 60);
  return (
    <Modal title={t('Choose a design to schedule')} onClose={onClose} size="lg">
      <input className="input" placeholder={t('Search your designs')} value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
      {!designs ? <div className="spinner" /> : !list.length ? <div className="empty">{t('No designs found.')}</div> : (
        <div className="network-grid" style={{ maxHeight: 420, overflow: 'auto' }}>
          {list.map((d) => (
            <button key={d.id} className="network-card" onClick={() => setPicked(d)}>
              <img src={thumbUrl(d.id, d.updated_at)} alt="" style={{ width: 44, height: 44, objectFit: 'cover', borderRadius: 6, background: 'var(--surface-3)' }} />
              <span className="t"><b>{d.title}</b><span>{fmtDate(d.updated_at, { month: 'short', day: 'numeric' })}</span></span>
            </button>
          ))}
        </div>
      )}
    </Modal>
  );
}
