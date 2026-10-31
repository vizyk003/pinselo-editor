/** Schedule a design to social networks (A16): pages, format, accounts, caption + hashtags, date/time + zone. */
import { CalendarClock, Hash, Plus, Trash2, X as XIcon } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router';
import { ApiError } from '../api';
import { Modal, useToast } from '../components/ui';
import { t, tn } from '../i18n';
import {
  charCount, composeText, isoToZoned, localTz, PROVIDER_COLOR, socialApi, suggestHashtags, timeZones, zonedToIso,
  type Provider, type SocialAccount, type SocialPost,
} from './api';
import { AccountBadge } from './AccountBadge';
import './planner.css';

export interface ScheduleProps {
  designId: string;
  title: string;
  pageCount: number;
  /** Text content of the design (hashtag suggestions). */
  texts?: string[];
  video?: boolean;
  post?: SocialPost;
  /** Prefill the date (Planner: clicked day). */
  initialAt?: Date;
  onClose: () => void;
  onSaved?: (p: SocialPost) => void;
}

function defaultWhen(initial?: Date): Date {
  const d = initial ? new Date(initial) : new Date(Date.now() + 3600_000);
  if (initial && initial.getHours() === 0 && initial.getMinutes() === 0) d.setHours(9, 0, 0, 0);
  else d.setMinutes(Math.ceil(d.getMinutes() / 15) * 15, 0, 0);
  return d;
}

export function ScheduleDialog({ designId, title, pageCount, texts = [], video, post, initialAt, onClose, onSaved }: ScheduleProps) {
  const toast = useToast();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [accounts, setAccounts] = useState<SocialAccount[] | null>(null);
  const [selected, setSelected] = useState<string[]>(post?.accounts ?? []);
  const [format, setFormat] = useState<'png' | 'jpg' | 'mp4'>(post?.format ?? (video ? 'mp4' : 'png'));
  const [pages, setPages] = useState<number[]>(post?.pages ?? []);
  const [caption, setCaption] = useState(post?.caption ?? '');
  const [tags, setTags] = useState<string[]>(post?.hashtags ?? []);
  const [tagInput, setTagInput] = useState('');
  const [tz, setTz] = useState(post?.timezone ?? localTz());
  const [when, setWhen] = useState(() => isoToZoned((post?.scheduledAt ? new Date(post.scheduledAt) : defaultWhen(initialAt)).toISOString(), post?.timezone ?? localTz()));
  const [options, setOptions] = useState<Record<string, unknown>>(post?.options ?? {});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  useEffect(() => {
    void socialApi.providers().then((r) => setProviders(r.providers)).catch(() => {});
    void socialApi.accounts().then((r) => {
      setAccounts(r.accounts);
      // New posts start with the local reminder selected when nothing else is connected.
      if (!post && r.accounts.length === 1) setSelected([r.accounts[0]!.id]);
    }).catch(() => setAccounts([]));
  }, [post]);

  const zones = useMemo(timeZones, []);
  const suggestions = useMemo(() => suggestHashtags(texts, title).filter((s) => !tags.some((x) => x.toLowerCase() === s.toLowerCase())), [texts, title, tags]);
  const text = composeText(caption, tags);
  const chosen = (accounts ?? []).filter((a) => selected.includes(a.id));
  const limitsOf = (a: SocialAccount) => providers.find((p) => p.id === a.provider)?.limits;
  const pageList = pages.length ? pages : [...Array(pageCount).keys()];

  const warnings: string[] = [];
  for (const a of chosen) {
    const l = limitsOf(a);
    const p = providers.find((x) => x.id === a.provider);
    if (!l || !p) continue;
    if (format === 'mp4' && !l.video) warnings.push(t('{network} doesn’t accept videos.', { network: p.label }));
    if (format !== 'mp4' && l.images === 0) warnings.push(t('{network} needs a video — choose MP4.', { network: p.label }));
    if (format !== 'mp4' && l.images > 0 && pageList.length > l.images) warnings.push(t('{network} shows at most {n} images; extra pages are left out.', { network: p.label, n: l.images }));
    if (charCount(text, a.provider) > l.maxChars) warnings.push(t('The caption is too long for {network}.', { network: p.label }));
    if (a.status === 'reauth') warnings.push(t('Reconnect {name} in Settings before it can post.', { name: a.displayName }));
  }
  const blocking = chosen.some((a) => {
    const l = limitsOf(a);
    return l && (charCount(text, a.provider) > l.maxChars || (format === 'mp4' && !l.video) || (format !== 'mp4' && l.images === 0));
  });

  const addTag = (raw: string) => {
    const v = raw.replace(/^#+/, '').replace(/[^\p{L}\p{N}_]/gu, '');
    if (v && !tags.some((x) => x.toLowerCase() === v.toLowerCase())) setTags([...tags, v]);
    setTagInput('');
  };

  const save = async (status: 'draft' | 'scheduled') => {
    setErr('');
    if (status === 'scheduled' && !selected.length) return setErr(t('Pick at least one account.'));
    let scheduledAt: string | null = null;
    if (when) {
      scheduledAt = zonedToIso(when, tz);
      if (status === 'scheduled' && new Date(scheduledAt).getTime() < Date.now() - 60_000) return setErr(t('Pick a time in the future.'));
    } else if (status === 'scheduled') return setErr(t('Pick a date and time.'));
    setBusy(true);
    try {
      const body = { pages: format === 'mp4' ? [] : pages, format, caption, hashtags: tags, accounts: selected, options, scheduledAt, timezone: tz, status };
      const r = post ? await socialApi.update(post.id, body) : await socialApi.create({ designId, ...body });
      toast(status === 'draft' ? t('Draft saved') : t('Scheduled for {when}', { when: new Date(r.post.scheduledAt!).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) }));
      onSaved?.(r.post);
      onClose();
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
    } finally {
      setBusy(false);
    }
  };

  const pinterest = chosen.find((a) => a.provider === 'pinterest');
  const youtube = chosen.some((a) => a.provider === 'youtube');
  const tiktok = chosen.some((a) => a.provider === 'tiktok');

  return (
    <Modal title={post ? t('Edit scheduled post') : t('Schedule to social')} onClose={onClose} size="lg">
      <div className="planner-form">
        <div className="field">
          <span>{t('Post to')}</span>
          {!accounts ? <div className="spinner" /> : (
            <div className="chips">
              {accounts.map((a) => (
                <button key={a.id} type="button" className={`chip acct-chip ${selected.includes(a.id) ? 'active' : ''}`} aria-pressed={selected.includes(a.id)}
                  onClick={() => setSelected(selected.includes(a.id) ? selected.filter((x) => x !== a.id) : [...selected, a.id])}>
                  <AccountBadge account={a} size={20} />
                  <span>{a.provider === 'local' ? t('Download & remind') : a.displayName}</span>
                </button>
              ))}
              <Link className="chip" to="/settings/accounts" onClick={onClose}><Plus size={14} /> {t('Connect accounts')}</Link>
            </div>
          )}
        </div>

        <div className="row wrap" style={{ gap: 16 }}>
          <div className="field">
            <span>{t('Format')}</span>
            <div className="segmented">
              {(['png', 'jpg', 'mp4'] as const).map((f) => (
                <button key={f} type="button" className={format === f ? 'on' : ''} onClick={() => setFormat(f)}>{f === 'mp4' ? t('Video (MP4)') : f.toUpperCase()}</button>
              ))}
            </div>
          </div>
          {format !== 'mp4' && pageCount > 1 && (
            <div className="field grow">
              <span>{t('Pages')} <span className="faint">{pageList.length > 1 ? t('— posted as a carousel') : ''}</span></span>
              <div className="chips">
                <button type="button" className={`chip ${!pages.length ? 'active' : ''}`} onClick={() => setPages([])}>{t('All ({n})', { n: pageCount })}</button>
                {[...Array(pageCount).keys()].map((i) => (
                  <button key={i} type="button" className={`chip page-chip ${pages.includes(i) ? 'active' : ''}`}
                    onClick={() => setPages(pages.includes(i) ? pages.filter((x) => x !== i) : [...pages, i].sort((a, b) => a - b))}>{i + 1}</button>
                ))}
              </div>
            </div>
          )}
        </div>

        <label className="field">
          <span>{t('Caption')}</span>
          <textarea className="input" rows={5} value={caption} onChange={(e) => setCaption(e.target.value)} placeholder={t('Write a caption…')} />
        </label>
        {chosen.length > 0 && (
          <div className="char-counts">
            {[...new Set(chosen.map((a) => a.provider))].map((pid) => {
              const p = providers.find((x) => x.id === pid);
              if (!p || pid === 'local') return null;
              const n = charCount(text, pid);
              return (
                <span key={pid} className={n > p.limits.maxChars ? 'over' : ''} title={p.label}>
                  <i style={{ background: PROVIDER_COLOR[pid] }} />{p.label} {n}/{p.limits.maxChars}
                </span>
              );
            })}
          </div>
        )}

        <div className="field">
          <span>{t('Hashtags')}</span>
          <div className="chips">
            {tags.map((h) => (
              <span key={h} className="chip active tag-chip">#{h}<button type="button" aria-label={t('Remove')} onClick={() => setTags(tags.filter((x) => x !== h))}><XIcon size={12} /></button></span>
            ))}
            <input className="input tag-input" value={tagInput} placeholder={t('Add a hashtag')} onChange={(e) => setTagInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ',' || e.key === ' ') {
                  e.preventDefault();
                  addTag(tagInput);
                }
              }} onBlur={() => tagInput && addTag(tagInput)} />
          </div>
          {suggestions.length > 0 && (
            <div className="chips suggestions">
              <span className="faint" style={{ fontSize: 12 }}><Hash size={12} /> {t('From your design:')}</span>
              {suggestions.map((s) => <button key={s} type="button" className="chip sm" onClick={() => addTag(s)}>#{s}</button>)}
            </div>
          )}
        </div>

        <div className="row wrap" style={{ gap: 12 }}>
          <label className="field grow">
            <span>{t('Date and time')}</span>
            <input className="input" type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} />
          </label>
          <label className="field grow">
            <span>{t('Time zone')}</span>
            {/* The wall time stays put when the zone changes: people pick the local time they mean. */}
            <select className="input" value={tz} onChange={(e) => setTz(e.target.value)}>
              {zones.map((z) => <option key={z} value={z}>{z.replace(/_/g, ' ')}</option>)}
            </select>
          </label>
        </div>

        {(pinterest || youtube || tiktok) && (
          <details className="net-options" open>
            <summary>{t('Network options')}</summary>
            {pinterest && (
              <label className="field">
                <span>{t('Pinterest board')}</span>
                <select className="input" value={String(options.pinterestBoardId ?? '')} onChange={(e) => setOptions({ ...options, pinterestBoardId: e.target.value || undefined })}>
                  <option value="">{t('First board')}</option>
                  {(pinterest.boards ?? []).map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
                </select>
              </label>
            )}
            {youtube && (
              <label className="field">
                <span>{t('YouTube title')}</span>
                <input className="input" maxLength={100} value={String(options.youtubeTitle ?? '')} placeholder={caption.split('\n')[0] || title} onChange={(e) => setOptions({ ...options, youtubeTitle: e.target.value })} />
              </label>
            )}
            {tiktok && (
              <label className="field">
                <span>{t('TikTok')}</span>
                <select className="input" value={String(options.tiktokMode ?? '')} onChange={(e) => setOptions({ ...options, tiktokMode: e.target.value || undefined })}>
                  <option value="">{t('Server default')}</option>
                  <option value="inbox">{t('Send to TikTok inbox (finish in the app)')}</option>
                  <option value="direct">{t('Post directly')}</option>
                </select>
              </label>
            )}
          </details>
        )}

        {warnings.length > 0 && <ul className="warn-list">{warnings.map((w) => <li key={w}>{w}</li>)}</ul>}
        {err && <p className="error" role="alert" style={{ margin: 0 }}>{err}</p>}

        <div className="row" style={{ justifyContent: 'flex-end', gap: 8 }}>
          {post && (
            <button type="button" className="btn ghost danger" disabled={busy} onClick={async () => {
              if (!confirm(t('Delete this scheduled post?'))) return;
              await socialApi.remove(post.id);
              onSaved?.({ ...post, status: 'draft', id: '' });
              onClose();
            }}><Trash2 size={15} /> {t('Delete')}</button>
          )}
          <div className="grow faint" style={{ fontSize: 12 }}>
            {format !== 'mp4' && tn(pageList.length, '{n} image', '{n} images')}
          </div>
          <button type="button" className="btn" disabled={busy} onClick={() => save('draft')}>{t('Save draft')}</button>
          <button type="button" className="btn primary" disabled={busy || blocking || !selected.length} onClick={() => save('scheduled')}>
            <CalendarClock size={16} /> {busy ? t('Saving…') : t('Schedule')}
          </button>
        </div>
      </div>
    </Modal>
  );
}
