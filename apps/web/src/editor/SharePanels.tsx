/**
 * Share dialog parts (plan 11 batch 2): the Add people combobox (g131), quick-action tiles (g133), template
 * link (g136), embed snippet (g135), QR code / send to phone, custom public link and "Remove everyone's
 * access" (g132). The dialog itself is ShareDialog in Dialogs.tsx.
 */
import QRCode from 'qrcode';
import { ArrowLeft, CodeXml, Copy, Download, LayoutTemplate, Link as LinkIcon, Presentation, QrCode, RefreshCw, Smartphone, Users, X } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { api, ApiError } from '../api';
import { sharingApi, type GroupHit, type PersonHit, type ShareRole } from '../api/sharing';
import { Avatar, useToast } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { embedSnippet, looksLikeEmail, previewSlug, splitTyped, templateLinkUrl } from './shareTools';
import './share.css';

export const ROLE_LABEL: Record<ShareRole, string> = { edit: tk('Can edit'), comment: tk('Can comment'), view: tk('Can view') };
const ROLES: ShareRole[] = ['edit', 'comment', 'view'];

export function RoleSelect({ value, onChange, label }: { value: ShareRole; onChange: (r: ShareRole) => void; label?: string }) {
  return (
    <select className="input" style={{ width: 130 }} value={value} onChange={(e) => onChange(e.target.value as ShareRole)} aria-label={label ?? t('Access level')}>
      {ROLES.map((r) => <option key={r} value={r}>{t(ROLE_LABEL[r])}</option>)}
    </select>
  );
}

export function GroupIcon() {
  return <span className="share-group-icon" aria-hidden><Users size={16} /></span>;
}

// ---------------------------------------------------------------- Add people (g131)

type Target =
  | { kind: 'person'; key: string; email: string; name: string; color: string }
  | { kind: 'group'; key: string; id: string; name: string; team: string; members: number };
interface Option { target: Target; label: string; sub: string; section: 'invite' | 'people' | 'groups' }

const personTarget = (email: string, name = email, color = 'var(--text-3)'): Target => ({ kind: 'person', key: `p:${email.toLowerCase()}`, email: email.toLowerCase(), name, color });

function shareError(x: Target, e: unknown): string {
  const who = x.kind === 'person' ? x.email : x.name;
  const code = e instanceof ApiError ? e.code : '';
  if (code === 'user_not_found') return t('{who} doesn’t have an account yet. Ask them to sign up, then share again.', { who });
  if (code === 'self') return t('You already own this design.');
  if (code === 'group_not_found') return t('{who}: you can only share with groups of your own teams.', { who });
  if (code === 'forbidden') return t('Only the owner can share this design.');
  return t('{who}: that didn’t work.', { who });
}

/**
 * Combobox over teammates, team groups and people you've shared with (suggestions when empty). Exact email
 * addresses still work for anyone else: type or paste them and they become chips.
 */
export function AddPeople({ designId, onShared }: { designId: string; onShared: () => void }) {
  const toast = useToast();
  const listId = useId();
  const input = useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const [picked, setPicked] = useState<Target[]>([]);
  const [role, setRole] = useState<ShareRole>('edit');
  const [open, setOpen] = useState(false);
  const [hits, setHits] = useState<{ people: PersonHit[]; groups: GroupHit[]; suggested: boolean } | null>(null);
  const [active, setActive] = useState(0);
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);

  useEffect(() => {
    if (!open) return;
    let live = true;
    const h = setTimeout(() => {
      sharingApi.people(text.trim(), designId).then((r) => { if (live) setHits(r); }).catch(() => { if (live) setHits(null); });
    }, text ? 150 : 0);
    return () => { live = false; clearTimeout(h); };
  }, [text, open, designId]);

  const options = useMemo<Option[]>(() => {
    const taken = new Set(picked.map((p) => p.key));
    const typed = text.trim().toLowerCase();
    const people = (hits?.people ?? []).filter((p) => !taken.has(`p:${p.email.toLowerCase()}`));
    const out: Option[] = [];
    if (looksLikeEmail(typed) && !taken.has(`p:${typed}`) && !people.some((p) => p.email.toLowerCase() === typed)) {
      out.push({ target: personTarget(typed), label: t('Invite {email}', { email: typed }), sub: t('Share by email address'), section: 'invite' });
    }
    for (const p of people) out.push({ target: personTarget(p.email, p.name, p.color), label: p.name, sub: p.teammate ? `${p.email} · ${t('Teammate')}` : p.email, section: 'people' });
    for (const g of hits?.groups ?? []) {
      if (!taken.has(`g:${g.id}`)) out.push({ target: { kind: 'group', key: `g:${g.id}`, id: g.id, name: g.name, team: g.team, members: g.members }, label: g.name, sub: `${g.team} · ${tn(g.members, '{n} member', '{n} members')}`, section: 'groups' });
    }
    return out;
  }, [hits, text, picked]);
  useEffect(() => setActive(0), [options.length]);

  const add = (targets: Target[]) => setPicked((cur) => [...cur, ...targets.filter((x) => !cur.some((c) => c.key === x.key))]);
  const pick = (o: Option) => {
    add([o.target]);
    setText('');
    input.current?.focus();
  };
  /** Turn typed/pasted email addresses into chips; anything else stays as search text. */
  const commitTyped = (value = text) => {
    const { emails, rest } = splitTyped(value);
    if (!emails.length) return false;
    add(emails.map((e) => personTarget(e)));
    setText(rest);
    return true;
  };

  const share = async () => {
    const typed = splitTyped(text).emails.map((e) => personTarget(e));
    const all = [...picked, ...typed.filter((x) => !picked.some((p) => p.key === x.key))];
    if (!all.length) return;
    setBusy(true);
    setErrors([]);
    const failed: Target[] = [];
    const errs: string[] = [];
    for (const x of all) {
      try {
        if (x.kind === 'person') await api.share(designId, x.email, role);
        else await sharingApi.shareGroup(designId, x.id, role);
      } catch (e) {
        failed.push(x);
        errs.push(shareError(x, e));
      }
    }
    setBusy(false);
    setPicked(failed);
    setText('');
    setErrors(errs);
    if (failed.length < all.length) {
      toast(t('Shared'));
      setHits(null);
      onShared();
    }
  };

  const activeId = open && options[active] ? `${listId}-${active}` : undefined;
  const sectionLabel: Record<Option['section'], string> = { invite: '', people: hits?.suggested ? t('Suggested') : t('People'), groups: t('Groups') };
  return (
    <div className="col" style={{ gap: 6 }}>
      <div className="share-add">
        <div className="share-combo">
          <div className="field-box" onMouseDown={(e) => { if (e.target === e.currentTarget) { e.preventDefault(); input.current?.focus(); } }}>
            {picked.map((p) => (
              <span key={p.key} className="share-pick" title={p.kind === 'person' ? p.email : `${p.name} · ${p.team}`}>
                {p.kind === 'group' && <Users size={12} />}
                <span>{p.kind === 'person' ? (p.name !== p.email ? p.name : p.email) : p.name}</span>
                <button type="button" aria-label={t('Remove {name}', { name: p.name })} onClick={() => setPicked(picked.filter((x) => x.key !== p.key))}><X size={12} /></button>
              </span>
            ))}
            <input
              ref={input}
              role="combobox"
              aria-expanded={open}
              aria-controls={listId}
              aria-autocomplete="list"
              aria-activedescendant={activeId}
              aria-label={t('Add people, groups or email addresses')}
              placeholder={picked.length ? '' : t('Add people, groups or emails')}
              value={text}
              // Suggestions open on click or typing, not on focus: the dialog focuses this box when it opens.
              onClick={() => setOpen(true)}
              onBlur={() => { setOpen(false); commitTyped(); }}
              onChange={(e) => {
                const v = e.target.value;
                setOpen(true);
                // A separator after an address turns it into a chip (also handles pasted lists).
                if (/[\s,;]$/.test(v) && commitTyped(v)) return;
                setText(v);
              }}
              onKeyDown={(e) => {
                // Opening the list starts at the first suggestion; once open, Down moves on.
                if (e.key === 'ArrowDown') { e.preventDefault(); setActive((a) => (open ? Math.min(a + 1, Math.max(0, options.length - 1)) : 0)); setOpen(true); }
                else if (e.key === 'ArrowUp') { e.preventDefault(); setActive((a) => Math.max(0, a - 1)); }
                else if (e.key === 'Enter') {
                  e.preventDefault();
                  if (open && options[active]) pick(options[active]!);
                  else if (!commitTyped() && !text.trim()) void share();
                } else if (e.key === 'Backspace' && !text && picked.length) setPicked(picked.slice(0, -1));
                else if (e.key === 'Escape' && open) {
                  // Close the list, not the whole dialog.
                  e.preventDefault();
                  e.stopPropagation();
                  setOpen(false);
                }
              }}
            />
          </div>
          {open && (
            <ul className="share-list" id={listId} role="listbox" aria-label={t('Suggestions')}>
              {sections(options).map(({ section, items }) => {
                const rendered = items.map(({ o, i }) => (
                  <li
                    key={o.target.key}
                    id={`${listId}-${i}`}
                    role="option"
                    aria-selected={i === active}
                    onMouseDown={(e) => e.preventDefault()}
                    onMouseEnter={() => setActive(i)}
                    onClick={() => pick(o)}
                  >
                    {o.target.kind === 'group' ? <GroupIcon /> : <Avatar name={o.target.name} color={o.target.color} />}
                    <div className="main"><div>{o.label}</div><div className="sub">{o.sub}</div></div>
                  </li>
                ));
                if (section === 'invite') return rendered;
                // Each section is a labelled group, so screen readers say which one an option is in.
                return (
                  <li key={section} role="group" aria-labelledby={`${listId}-${section}`}>
                    <div className="label" id={`${listId}-${section}`}>{sectionLabel[section]}</div>
                    <ul role="none" className="share-group">{rendered}</ul>
                  </li>
                );
              })}
              {!options.length && <li className="empty-hint" role="presentation">{text.trim() ? t('No matches. Type a full email address to share with anyone.') : t('Type a name, group or email address.')}</li>}
            </ul>
          )}
        </div>
        <RoleSelect value={role} onChange={setRole} />
        <button className="btn primary" disabled={busy || (!picked.length && !splitTyped(text).emails.length)} onClick={() => void share()}>{busy ? t('Sharing…') : t('Share')}</button>
      </div>
      {errors.map((e) => <div key={e} className="form-error">{e}</div>)}
    </div>
  );
}

/** Consecutive options of the same section, with their overall index (the active-descendant numbering). */
function sections(options: Option[]): Array<{ section: Option['section']; items: Array<{ o: Option; i: number }> }> {
  const out: Array<{ section: Option['section']; items: Array<{ o: Option; i: number }> }> = [];
  options.forEach((o, i) => {
    const last = out[out.length - 1];
    if (last && last.section === o.section) last.items.push({ o, i });
    else out.push({ section: o.section, items: [{ o, i }] });
  });
  return out;
}

// ---------------------------------------------------------------- quick actions (g133)

export type SharePane = 'template' | 'embed' | 'qr' | 'phone';

export function QuickActions({ onDownload, onPresent, onCopyLink, onPane, focusTile }: {
  onDownload: () => void; onPresent: () => void; onCopyLink: () => void; onPane: (p: SharePane) => void;
  /** Back from a pane: the tile that opened it takes the focus again. */
  focusTile?: SharePane | null;
}) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});
  // These tiles mount again when a pane closes (the dialog swaps its content), so this runs once per return.
  useEffect(() => {
    if (focusTile) refs.current[focusTile]?.focus();
  }, [focusTile]);
  const tiles: Array<{ id: string; icon: ReactNode; label: string; run: () => void }> = [
    { id: 'download', icon: <Download size={18} />, label: t('Download'), run: onDownload },
    { id: 'present', icon: <Presentation size={18} />, label: t('Present'), run: onPresent },
    { id: 'copy', icon: <LinkIcon size={18} />, label: t('Copy link'), run: onCopyLink },
    { id: 'template', icon: <LayoutTemplate size={18} />, label: t('Template link'), run: () => onPane('template') },
    { id: 'embed', icon: <CodeXml size={18} />, label: t('Embed'), run: () => onPane('embed') },
    { id: 'qr', icon: <QrCode size={18} />, label: t('QR code'), run: () => onPane('qr') },
    { id: 'phone', icon: <Smartphone size={18} />, label: t('Send to phone'), run: () => onPane('phone') },
  ];
  return (
    <div className="share-tiles" role="group" aria-label={t('Share actions')}>
      {tiles.map((x) => (
        <button key={x.id} type="button" onClick={x.run} ref={(el) => { refs.current[x.id] = el; }}><span className="ic">{x.icon}</span>{x.label}</button>
      ))}
    </div>
  );
}

export function PaneHead({ title, onBack }: { title: string; onBack: () => void }) {
  return (
    <div className="share-pane-head">
      <button className="btn ghost sm icon" onClick={onBack} aria-label={t('Back')} autoFocus><ArrowLeft size={16} /></button>
      <h3>{title}</h3>
    </div>
  );
}

function CopyRow({ value, label, copied }: { value: string; label: string; copied: string }) {
  const toast = useToast();
  return (
    <div className="row">
      <input className="input" dir="ltr" readOnly value={value} onFocus={(e) => e.target.select()} aria-label={label} />
      <button className="btn" onClick={() => { void navigator.clipboard.writeText(value); toast(copied); }}><Copy size={16} /> {t('Copy')}</button>
    </div>
  );
}

// ---------------------------------------------------------------- template link (g136)

export function TemplateLinkPane({ designId, owner, initial }: { designId: string; owner: boolean; initial: { token: string | null; uses: number } | null }) {
  const toast = useToast();
  const [link, setLink] = useState(initial);
  const [busy, setBusy] = useState(false);
  const run = (p: Promise<unknown>) => {
    setBusy(true);
    p.catch((e) => toast(e instanceof ApiError && e.message !== e.code ? e.message : t('That didn’t work.'))).finally(() => setBusy(false));
  };
  if (!owner) return <p className="muted" style={{ margin: 0 }}>{t('Only the owner can create a template link. You can still make your own copy from the File menu.')}</p>;
  const intro = <p className="muted" style={{ margin: 0 }}>{t('Anyone who opens a template link gets their own copy of this design to edit. They can’t see or change your original.')}</p>;
  if (!link?.token) {
    return (
      <>
        {intro}
        <button className="btn primary" disabled={busy} onClick={() => run(sharingApi.enableTemplateLink(designId).then(setLink))}><LayoutTemplate size={16} /> {t('Create template link')}</button>
      </>
    );
  }
  return (
    <>
      {intro}
      <CopyRow value={templateLinkUrl(location.origin, link.token)} label={t('Template link')} copied={t('Template link copied')} />
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{tn(link.uses, 'Used {n} time', 'Used {n} times')} · {t('People sign in to get their copy.')}</p>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <button className="btn sm" disabled={busy} onClick={() => {
          if (!confirm(t('Reset the template link? The current link stops working.'))) return;
          run(sharingApi.enableTemplateLink(designId, true).then(setLink));
        }}><RefreshCw size={14} /> {t('Reset link')}</button>
        <button className="btn sm danger" disabled={busy} onClick={() => run(sharingApi.disableTemplateLink(designId).then(() => { setLink({ token: null, uses: link.uses }); toast(t('Template link turned off')); }))}>{t('Turn off')}</button>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- embed (g135)

export function EmbedPane({ pub, owner, hasPassword, viewLink, w, h, title, onTurnOn }: {
  pub: boolean; owner: boolean; hasPassword: boolean; viewLink: string; w: number; h: number; title: string; onTurnOn: () => Promise<void>;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  if (!pub) {
    return (
      <>
        <p className="muted" style={{ margin: 0 }}>{t('Embedding shows the design on other websites through its public view link, which is off.')}</p>
        {owner
          ? <button className="btn primary" disabled={busy} onClick={() => { setBusy(true); void onTurnOn().finally(() => setBusy(false)); }}>{t('Turn on public view link')}</button>
          : <p className="faint" style={{ margin: 0, fontSize: 13 }}>{t('Ask the owner to turn on the public view link.')}</p>}
      </>
    );
  }
  const code = embedSnippet(viewLink, w, h, title);
  return (
    <>
      <p className="muted" style={{ margin: 0 }}>{t('Paste this code into your website or blog. The embed resizes with the page and always shows the latest version.')}</p>
      <textarea className="input share-code" dir="ltr" readOnly value={code} onFocus={(e) => e.target.select()} aria-label={t('Embed code')} />
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <button className="btn primary" onClick={() => { void navigator.clipboard.writeText(code); toast(t('Embed code copied')); }}><Copy size={16} /> {t('Copy embed code')}</button>
        <a className="btn" href={`${viewLink}?embed=1`} target="_blank" rel="noreferrer">{t('Preview')}</a>
      </div>
      {hasPassword && <p className="form-error" style={{ margin: 0 }}>{t('This link has a password, so the embed asks visitors for it. Sites that read oEmbed can’t show it.')}</p>}
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Tools that support oEmbed (such as WordPress) embed it when you paste the public link.')}</p>
    </>
  );
}

// ---------------------------------------------------------------- QR code / send to phone (g133)

export function QrPane({ url, mode, pub, title }: { url: string; mode: 'qr' | 'phone'; pub: boolean; title: string }) {
  const [img, setImg] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    QRCode.toDataURL(url, { margin: 1, width: 440 }).then((d) => { if (live) setImg(d); }).catch(() => { if (live) setImg(null); });
    return () => { live = false; };
  }, [url]);
  return (
    <div className="share-qr">
      <p className="muted" style={{ margin: 0, textAlign: 'center' }}>
        {mode === 'phone' ? t('Point your phone’s camera at the code to open this design there.') : t('Anyone who scans this code opens the link below.')}
      </p>
      {img ? <img src={img} alt={t('QR code for {url}', { url })} /> : <div className="share-qr-ph" style={{ width: 220, height: 220 }} />}
      <code className="faint" dir="ltr" style={{ fontSize: 12, wordBreak: 'break-all', textAlign: 'center' }}>{url}</code>
      <p className="faint" style={{ margin: 0, fontSize: 12, textAlign: 'center' }}>
        {pub ? t('This is the public view link: no sign-in needed.') : t('This is the collaboration link: people sign in and need access.')}
      </p>
      {mode === 'qr' && img && <a className="btn sm" href={img} download={`${title.replace(/[^\p{L}\p{N}\- ]+/gu, '').trim() || 'design'} QR.png`}><Download size={14} /> {t('Download QR code')}</a>}
    </div>
  );
}

// ---------------------------------------------------------------- custom public link (g131)

const SLUG_ERRORS: Record<string, string> = {
  slug_length: tk('Use 3 to 60 characters.'),
  slug_chars: tk('Use only letters, numbers and hyphens.'),
  slug_reserved: tk('That link is reserved. Try another one.'),
  slug_id_like: tk('Add a hyphen or pick a different link.'),
  slug_taken: tk('That link is already taken.'),
};

export function SlugEditor({ designId, slug, onSaved }: { designId: string; slug: string | null; onSaved: (s: string | null) => void }) {
  const toast = useToast();
  const [v, setV] = useState(slug ?? '');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const save = (next: string | null) => {
    setBusy(true);
    setErr('');
    sharingApi.setSlug(designId, next).then((r) => {
      setV(r.slug ?? '');
      onSaved(r.slug);
      toast(r.slug ? t('Custom link saved') : t('Custom link removed'));
    }).catch((e) => setErr(e instanceof ApiError && SLUG_ERRORS[e.code] ? t(SLUG_ERRORS[e.code]!) : t('That didn’t work.'))).finally(() => setBusy(false));
  };
  return (
    <>
      <form className="share-slug" onSubmit={(e) => { e.preventDefault(); save(v); }}>
        {/* A URL reads left to right in every language. */}
        <span className="share-slug-url" dir="ltr">
          <span className="prefix">{location.host}/view/</span>
          <input className="input" value={v} onChange={(e) => setV(previewSlug(e.target.value))} placeholder={t('your-custom-link')} aria-label={t('Custom link')} maxLength={60} aria-invalid={Boolean(err)} />
        </span>
        <button className="btn sm" disabled={busy || v.replace(/-+$/, '') === (slug ?? '') || v.length < 3}>{t('Save')}</button>
        {slug && <button type="button" className="btn sm ghost" disabled={busy} onClick={() => save(null)}>{t('Remove')}</button>}
      </form>
      {err && <div className="form-error">{err}</div>}
    </>
  );
}

// ---------------------------------------------------------------- remove everyone's access (g132)

export function RemoveEveryoneConfirm({ designId, team, onCancel, onDone }: { designId: string; team: string | null; onCancel: () => void; onDone: () => void }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  return (
    <div className="share-confirm" role="alertdialog" aria-labelledby={titleId}>
      <p id={titleId}><b>{t('Remove everyone’s access?')}</b></p>
      <p>{t('Everyone you shared with (people and groups) loses access, the collaboration link changes to “Only people with access”, and the public view link and template link are turned off.')}</p>
      {team && <p className="faint">{t('Members of {team} keep their team access.', { team })}</p>}
      <div className="row" style={{ justifyContent: 'flex-end' }}>
        <button className="btn sm" onClick={onCancel} autoFocus>{t('Cancel')}</button>
        <button className="btn sm" style={{ background: 'var(--danger)', color: '#fff', borderColor: 'transparent' }} disabled={busy} onClick={() => {
          setBusy(true);
          sharingApi.makePrivate(designId).then(() => { toast(t('Everyone’s access was removed')); onDone(); })
            .catch(() => toast(t('That didn’t work.'))).finally(() => setBusy(false));
        }}>{busy ? t('Removing…') : t('Remove access')}</button>
      </div>
    </div>
  );
}
