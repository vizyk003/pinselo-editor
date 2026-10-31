import {
  formatDims, lockedSide, PRESET_CATEGORIES, presetsInCategory, pxToUnit, searchPresets, SIZE_PRESETS, sortedPages, unitToPx, videoSlots, type PresetCategoryId, type Unit,
} from '@oc/engine';
import { Building2, Download, Eye, Globe, Link as LinkIcon, Lock, Printer, Share2, Trash2, Unlock, UserX } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { tabPanelProps, tabProps } from '../components/tabs';
import { ResizeCopies } from './ResizeCopies';
import { accessApi, api, contentApi, download, publicApi, type AccessRequest, type Credit, type Role, type User } from '../api';
import { Avatar, interp, Modal, SearchField, useToast } from '../components/ui';
import { fmtDate, t, tk, tn } from '../i18n';
import { track } from '../lib/analytics';
import { sharingApi, type ShareRole, type SharingState } from '../api/sharing';
import { useDoc, useEd, useEditorState } from './context';
import { ViewInsightsLine } from './SiteInsights';
import { ApprovalSection } from './Approval';
import { canShareNatively, shareLink } from '../native';
import { PrintDialog } from '../print/PrintDialog';
import { StandalonePreflight } from '../print/Preflight';
import {
  AddPeople, EmbedPane, GroupIcon, PaneHead, QrPane, QuickActions, RemoveEveryoneConfirm, ROLE_LABEL, RoleSelect, SlugEditor, TemplateLinkPane, type SharePane,
} from './SharePanels';
import { viewUrl } from './shareTools';

export function ShareDialog({ onClose }: { onClose: () => void }) {
  const { design, role, openDialog, setPresenting } = useEd();
  const toast = useToast();
  const [data, setData] = useState<{ linkAccess: string; owner: User; people: Array<User & { role: Role }> } | null>(null);
  // Teams, groups, slug, template link and visitor count (plan 11 batch 2).
  const [extra, setExtra] = useState<SharingState | null>(null);
  const owner = role === 'owner';
  const [pub, setPub] = useState(Boolean(design.public_view));
  const [pubCfg, setPubCfg] = useState<{ expiresAt: string | null; hasPassword: boolean }>({ expiresAt: null, hasPassword: false });
  const [newPw, setNewPw] = useState('');
  const [requests, setRequests] = useState<AccessRequest[]>([]);
  const [pane, setPane] = useState<SharePane | null>(null);
  // Keyboard focus after a sub-pane or the confirm box closes: back on the control that opened it, not lost to <body>.
  const [lastPane, setLastPane] = useState<SharePane | null>(null);
  const refocusRemove = useRef(false);
  const [confirmPrivate, setConfirmPrivate] = useState(false);
  const reload = () => Promise.all([api.shares(design.id).then(setData), sharingApi.state(design.id).then(setExtra).catch(() => {})]);
  useEffect(() => {
    void reload();
    if (owner) void publicApi.settings(design.id).then((r) => { setPubCfg(r); setPub(r.enabled); }).catch(() => {});
    if (role === 'owner' || role === 'edit') void accessApi.list(design.id).then((r) => setRequests(r.requests)).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const expiryChoice = (days: number | null) => {
    const expiresAt = days === null ? null : new Date(Date.now() + days * 86400_000).toISOString();
    void publicApi.setPublic(design.id, true, { expiresAt }).then(() => setPubCfg((c) => ({ ...c, expiresAt })));
  };
  const setPublic = async (on: boolean) => {
    try {
      await publicApi.setPublic(design.id, on);
      setPub(on);
    } catch (x) {
      toast((x as Error).message);
    }
  };
  const link = `${location.origin}/design/${design.id}`;
  const publicLink = viewUrl(location.origin, design.id, extra?.slug);
  // Embeds point at the design id, which never changes hands (a custom slug can be changed or removed).
  const embedLink = viewUrl(location.origin, design.id);
  // Quick actions share the public view link when it's on, else the collaboration link.
  const bestLink = pub ? publicLink : link;
  const anyoneHasAccess = Boolean(data?.people.length || extra?.groups.length || (data && data.linkAccess !== 'restricted') || pub || extra?.templateLink?.token);

  if (pane) {
    const titles: Record<SharePane, string> = { template: t('Template link'), embed: t('Embed'), qr: t('QR code'), phone: t('Send to phone') };
    return (
      <Modal title={t('Share this design')} onClose={onClose} size="md">
        <PaneHead title={titles[pane]} onBack={() => { setLastPane(pane); setPane(null); void reload(); }} />
        {pane === 'template' && <TemplateLinkPane designId={design.id} owner={owner} initial={extra?.templateLink ?? null} />}
        {pane === 'embed' && (
          <EmbedPane pub={pub} owner={owner} hasPassword={pubCfg.hasPassword} viewLink={embedLink} w={design.width} h={design.height} title={design.title} onTurnOn={() => setPublic(true)} />
        )}
        {(pane === 'qr' || pane === 'phone') && <QrPane url={bestLink} mode={pane} pub={pub} title={design.title} />}
      </Modal>
    );
  }

  return (
    <Modal title={t('Share this design')} onClose={onClose} size="md">
      {extra?.visitors != null && (pub || extra.visitors > 0) && (
        <div className="share-head">
          <span className="grow" />
          <span className="visitors" title={t('Unique visitors of the public view link')}><Eye size={14} /> {tn(extra.visitors, '{n} visitor', '{n} visitors')}</span>
        </div>
      )}
      {owner && <AddPeople designId={design.id} onShared={() => void reload()} />}
      {requests.length > 0 && (
        <div className="col" style={{ gap: 6 }}>
          <h4 style={{ margin: 0 }} className="muted">{t('Access requests')}</h4>
          {requests.map((r) => (
            <div key={r.id} className="row">
              <Avatar name={r.name} color={r.color} />
              <div className="grow"><b>{r.name}</b><div className="faint" style={{ fontSize: 12 }}>{r.email}{r.message ? ` · “${r.message}”` : ''}</div></div>
              <button className="btn sm primary" onClick={async () => { await accessApi.decide(r.id, true); setRequests(requests.filter((x) => x.id !== r.id)); void reload(); }}>{({ edit: t('Allow edit'), comment: t('Allow comment'), view: t('Allow view') } as Record<string, string>)[r.role]}</button>
              <button className="btn sm ghost" onClick={async () => { await accessApi.decide(r.id, false); setRequests(requests.filter((x) => x.id !== r.id)); }}>{t('Decline')}</button>
            </div>
          ))}
        </div>
      )}
      <h4 style={{ margin: 0 }} className="muted">{t('People with access')}</h4>
      {data && (
        <div className="col">
          <div className="row"><Avatar name={data.owner.name} color={data.owner.color} /><div className="grow"><b>{data.owner.name}</b><div className="faint" style={{ fontSize: 12 }}>{data.owner.email}</div></div><span className="badge">{t('Owner')}</span></div>
          {extra?.team && (
            <div className="row">
              <span className="share-group-icon" aria-hidden><Building2 size={16} /></span>
              <div className="grow">
                <b>{t('Everyone at {team}', { team: extra.team.name })}</b>
                <div className="faint" style={{ fontSize: 12 }}>{extra.team.access ? t('One level for every member; team admins can always manage it.') : t('Designers can edit, members can comment.')}</div>
              </div>
              {owner ? (
                <select className="input" style={{ width: 150 }} aria-label={t('Team access')} value={extra.team.access ?? 'default'} onChange={async (e) => {
                  await sharingApi.setTeamAccess(design.id, e.target.value === 'default' ? null : (e.target.value as ShareRole));
                  void reload();
                }}>
                  <option value="default">{t('By team role')}</option>
                  <option value="edit">{t('Can edit')}</option><option value="comment">{t('Can comment')}</option><option value="view">{t('Can view')}</option>
                </select>
              ) : <span className="badge">{extra.team.access ? t(ROLE_LABEL[extra.team.access]) : t('By team role')}</span>}
            </div>
          )}
          {extra?.groups.map((g) => (
            <div key={g.id} className="row">
              <GroupIcon />
              <div className="grow"><b>{g.name}</b><div className="faint" style={{ fontSize: 12 }}>{g.team} · {tn(g.members, '{n} member', '{n} members')}</div></div>
              {owner ? (
                <>
                  <RoleSelect value={g.role} label={t('Access for {name}', { name: g.name })} onChange={async (r) => { await sharingApi.shareGroup(design.id, g.id, r); void reload(); }} />
                  <button className="btn ghost sm icon" title={t('Remove')} aria-label={t('Remove {name}', { name: g.name })} onClick={async () => { await sharingApi.unshareGroup(design.id, g.id); void reload(); }}><Trash2 size={14} /></button>
                </>
              ) : <span className="badge">{t(ROLE_LABEL[g.role])}</span>}
            </div>
          ))}
          {data.people.map((p) => (
            <div key={p.id} className="row">
              <Avatar name={p.name} color={p.color} />
              <div className="grow"><b>{p.name}</b><div className="faint" style={{ fontSize: 12 }}>{p.email}</div></div>
              {owner ? (
                <>
                  <RoleSelect value={p.role as ShareRole} label={t('Access for {name}', { name: p.name })} onChange={async (r) => { await api.share(design.id, p.email, r); void reload(); }} />
                  <button className="btn ghost sm icon" title={t('Remove')} aria-label={t('Remove {name}', { name: p.name })} onClick={async () => { await api.unshare(design.id, p.id); void reload(); }}><Trash2 size={14} /></button>
                  <button className="btn ghost sm" title={t('Make {name} the owner', { name: p.name })} onClick={async () => {
                    if (!confirm(t('Transfer ownership to {name}? You will keep edit access.', { name: p.name }))) return;
                    await accessApi.transfer(design.id, p.id);
                    toast(t('Ownership transferred'));
                    onClose();
                  }}>{t('Make owner')}</button>
                </>
              ) : <span className="badge">{({ edit: t('Can edit'), comment: t('Can comment'), view: t('Can view') } as Record<string, string>)[p.role] ?? p.role}</span>}
            </div>
          ))}
        </div>
      )}
      {owner && anyoneHasAccess && !confirmPrivate && (
        <button className="btn ghost sm danger" style={{ alignSelf: 'flex-start' }} onClick={() => setConfirmPrivate(true)}
          ref={(el) => { if (el && refocusRemove.current) { refocusRemove.current = false; el.focus(); } }}><UserX size={14} /> {t('Remove everyone’s access')}</button>
      )}
      {confirmPrivate && (
        <RemoveEveryoneConfirm designId={design.id} team={extra?.team?.name ?? null} onCancel={() => { refocusRemove.current = true; setConfirmPrivate(false); }} onDone={() => { setConfirmPrivate(false); setPub(false); void reload(); }} />
      )}
      <div className="hsep" />
      <h4 style={{ margin: 0 }} className="muted">{t('Collaboration link')}</h4>
      <div className="row">
        {data?.linkAccess === 'restricted' ? <Lock size={18} /> : <Globe size={18} />}
        <select className="input" disabled={!owner} value={data?.linkAccess ?? 'restricted'} onChange={async (e) => { await api.updateDesign(design.id, { linkAccess: e.target.value }); void reload(); }}>
          <option value="restricted">{t('Only people with access')}</option>
          <option value="view">{t('Anyone with the link can view')}</option>
          <option value="comment">{t('Anyone with the link can comment')}</option>
          <option value="edit">{t('Anyone with the link can edit')}</option>
        </select>
      </div>
      <div className="row">
        <input className="input" dir="ltr" readOnly value={link} onFocus={(e) => e.target.select()} aria-label={t('Collaboration link')} />
        <button className="btn" onClick={() => { void navigator.clipboard.writeText(link); toast(t('Link copied')); }}><LinkIcon size={16} /> {t('Copy')}</button>
        {canShareNatively() && <button className="btn" onClick={() => void shareLink(link, design.title)}><Share2 size={16} /> {t('Share…')}</button>}
      </div>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t("People opening the link need to sign in. Viewers can't change anything.")}</p>
      <div className="hsep" />
      <h4 style={{ margin: 0 }} className="muted">{t('Public view link')}</h4>
      <label className="row" style={{ fontSize: 13 }}>
        <input type="checkbox" disabled={!owner} checked={Boolean(pub)} onChange={(e) => void setPublic(e.target.checked)} />
        {t('Anyone on the internet with the link can view (no sign-in)')}
      </label>
      {pub && (
        <div className="row">
          <input className="input" dir="ltr" readOnly value={publicLink} onFocus={(e) => e.target.select()} aria-label={t('Public view link')} />
          <button className="btn" onClick={() => { void navigator.clipboard.writeText(publicLink); toast(t('Public link copied')); }}><LinkIcon size={16} /> {t('Copy')}</button>
        </div>
      )}
      {pub && owner && extra && <SlugEditor designId={design.id} slug={extra?.slug ?? null} onSaved={(slug) => setExtra((x) => (x ? { ...x, slug } : x))} />}
      {pub && (owner || role === 'edit') && <ViewInsightsLine designId={design.id} />}
      {pub && owner && (
        <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>
          <label className="row" style={{ gap: 6, fontSize: 13 }}>{t('Expires')}
            <select className="input" style={{ width: 'auto' }} value={pubCfg.expiresAt ? 'set' : 'never'} onChange={(e) => expiryChoice(e.target.value === 'never' ? null : +e.target.value)}>
              <option value="never">{t('Never')}</option>
              {pubCfg.expiresAt && <option value="set">{fmtDate(pubCfg.expiresAt, { dateStyle: 'medium', timeStyle: 'short' })}</option>}
              <option value="1">{t('In 1 day')}</option><option value="7">{t('In 7 days')}</option><option value="30">{t('In 30 days')}</option>
            </select>
          </label>
          {pubCfg.hasPassword ? (
            <button className="btn sm" onClick={async () => { await publicApi.setPublic(design.id, true, { password: null }); setPubCfg((c) => ({ ...c, hasPassword: false })); toast(t('Password removed')); }}><Lock size={14} /> {t('Remove password')}</button>
          ) : (
            <form className="row" style={{ gap: 6 }} onSubmit={async (e) => { e.preventDefault(); await publicApi.setPublic(design.id, true, { password: newPw }); setNewPw(''); setPubCfg((c) => ({ ...c, hasPassword: true })); toast(t('Password set')); }}>
              <input className="input" type="password" style={{ width: 160 }} minLength={4} placeholder={t('Add a password')} value={newPw} onChange={(e) => setNewPw(e.target.value)} aria-label={t('Link password')} />
              <button className="btn sm" disabled={newPw.length < 4}>{t('Set')}</button>
            </form>
          )}
        </div>
      )}
      <div className="hsep" />
      <QuickActions
        onDownload={() => openDialog('export')}
        onPresent={() => { onClose(); setPresenting(true); }}
        onCopyLink={() => { void navigator.clipboard.writeText(bestLink); toast(pub ? t('Public link copied') : t('Link copied')); }}
        onPane={setPane}
        focusTile={lastPane}
      />
      <ApprovalSection />
    </Modal>
  );
}

/** Download (Canva-style panel under the top-bar Download button): see DownloadPanel.tsx. */
export { DownloadPanel as ExportDialog } from './DownloadPanel';

const RESIZE_TABS = ['resize', 'copies'] as const;

export function ResizeDialog({ onClose }: { onClose: () => void }) {
  const { editor, readOnly } = useEd();
  const tabsId = useId();
  const [tab, setTab] = useState<(typeof RESIZE_TABS)[number]>('resize');
  const doc = useDoc();
  const toast = useToast();
  const { dims } = doc.meta;
  const [unit, setUnit] = useState<Unit>(dims.unit);
  const [w, setW] = useState(+pxToUnit(dims.w, dims.unit).toFixed(2));
  const [h, setH] = useState(+pxToUnit(dims.h, dims.unit).toFixed(2));
  const [lock, setLock] = useState(false);
  const [scaleContent, setScaleContent] = useState(true);
  // Format list: a search across every preset, or one category (g08 categories) at a time.
  const [q, setQ] = useState('');
  const [cat, setCat] = useState<PresetCategoryId | 'all'>('all');
  const formats = q.trim() ? searchPresets(q, t) : cat === 'all' ? SIZE_PRESETS : presetsInCategory(cat);
  const setSide = (side: 'w' | 'h', v: number) => {
    if (side === 'w') setW(v); else setH(v);
    if (lock && v > 0 && w > 0 && h > 0) (side === 'w' ? setH : setW)(lockedSide(side, v, w / h));
  };
  const apply = (pw: number, ph: number, u: Unit) => {
    editor.resizeDesign(pw, ph, u, scaleContent);
    editor.zoomToFit();
    toast(t('Resized to {size}', { size: formatDims(pw, ph, u) }));
    onClose();
  };
  return (
    <Modal title={t('Resize design')} onClose={onClose} size="md">
      <p className="muted" style={{ margin: 0 }}>{t('Current size: {size}', { size: formatDims(dims.w, dims.h, dims.unit) })}</p>
      {!readOnly && (
        <div className="lib-tabs" role="tablist" aria-label={t('Resize design')}>
          {RESIZE_TABS.map((x) => (
            <button key={x} {...tabProps(tabsId, RESIZE_TABS, tab, setTab, x)} className={`btn sm${tab === x ? ' primary' : ' ghost'}`}>
              {x === 'resize' ? t('Resize this design') : t('Copy and resize')}
            </button>
          ))}
        </div>
      )}
      {tab === 'copies' ? <div {...tabPanelProps(tabsId, tab)}><ResizeCopies onClose={onClose} /></div> : <div {...tabPanelProps(tabsId, tab)} className="col">
      <div className="row">
        <input className="input" type="number" value={w} onChange={(e) => setSide('w', +e.target.value)} aria-label={t('Width')} />
        <button className={`btn icon sm${lock ? ' primary' : ''}`} aria-pressed={lock} title={lock ? t('Unlock aspect ratio') : t('Lock aspect ratio')} onClick={() => setLock(!lock)}>
          {lock ? <Lock size={14} /> : <Unlock size={14} />}
        </button>
        <input className="input" type="number" value={h} onChange={(e) => setSide('h', +e.target.value)} aria-label={t('Height')} />
        <select className="input" style={{ width: 80 }} value={unit} onChange={(e) => setUnit(e.target.value as Unit)} aria-label={t('Unit')}>{(['px', 'in', 'mm', 'cm'] as const).map((u) => <option key={u}>{u}</option>)}</select>
      </div>
      <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={scaleContent} onChange={(e) => setScaleContent(e.target.checked)} /> {t('Scale content to fit the new size')}</label>
      <button className="btn primary" onClick={() => apply(unitToPx(w, unit), unitToPx(h, unit), unit)} disabled={!(w > 0 && h > 0)}>{t('Resize')}</button>
      <div className="hsep" />
      <h4 style={{ margin: 0 }} className="muted">{t('Or pick a format')}</h4>
      <div className="row">
        <SearchField className="grow" value={q} onChange={setQ} placeholder={t('Search sizes')} onClear={() => setQ('')} />
        <select className="input" style={{ width: 170 }} value={cat} onChange={(e) => { setQ(''); setCat(e.target.value as typeof cat); }} aria-label={t('Category')}>
          <option value="all">{t('All sizes')}</option>
          {PRESET_CATEGORIES.filter((c) => c.group).map((c) => <option key={c.id} value={c.id}>{t(c.label)}</option>)}
        </select>
      </div>
      <div className="preset-list" style={{ maxHeight: 260, overflow: 'auto' }}>
        {formats.map((p) => (
          <button key={p.id} onClick={() => apply(p.w, p.h, p.unit)}>
            <span>{t(p.label)}</span><span className="faint" style={{ fontSize: 12 }}>{formatDims(p.w, p.h, p.unit)}</span>
          </button>
        ))}
        {!formats.length && <span className="faint" style={{ fontSize: 13 }}>{t('No sizes match “{q}”.', { q: q.trim() })}</span>}
      </div>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Tip: “Copy and resize” keeps this design and makes copies in other sizes.')}</p>
      </div>}
    </Modal>
  );
}

const SHORTCUTS: Array<[string, string]> = [
  [tk('Undo / Redo'), '⌘Z / ⇧⌘Z'], [tk('Copy / Cut / Paste'), '⌘C / ⌘X / ⌘V'], [tk('Duplicate'), '⌘D'], [tk('Delete'), '⌫'],
  [tk('Select all'), '⌘A'], [tk('Group / Ungroup'), '⌘G / ⇧⌘G'], [tk('Bring forward / to front'), '⌘] / ⌥⌘]'], [tk('Send backward / to back'), '⌘[ / ⌥⌘['],
  [tk('Nudge (10px with ⇧)'), '← ↑ → ↓'], [tk('Lock / unlock'), '⌥⇧L'], [tk('Edit text / exit'), 'Enter / Esc'], [tk('Bold / Italic / Underline'), '⌘B / ⌘I / ⌘U'],
  [tk('Zoom in / out / fit'), '⌘+ / ⌘− / ⌘0'], [tk('Pan'), tk('Space + drag')], [tk('New page'), '⌘Enter'], [tk('Select first element (canvas focused)'), 'Enter'], [tk('Next / previous element; past the last, leave the canvas'), 'Tab / ⇧Tab'], [tk('Deselect, then leave the canvas'), tk('Esc, then Tab')],
  [tk('Flip horizontal / vertical'), '⌥⇧H / ⌥⇧V'], [tk('Present'), '⌥⌘P'], [tk('Keyboard shortcuts'), '?'],
  [tk('Command palette'), '/'], [tk('Add text / rectangle / circle / line'), 'T / R / C / L'], [tk('Delete page'), '⌘⌫'], [tk('Add a link'), '⌘K'],
  [tk('Bigger / smaller text'), '⇧⌘> / ⇧⌘<'], [tk('Align text left / centre / right / justify'), '⇧⌘L / C / R / J'], [tk('Uppercase'), '⇧⌘K'],
  [tk('Numbered / bulleted list'), '⇧⌘7 / ⇧⌘8'], [tk('Copy / paste style'), '⌥⌘C / ⌥⌘V'], [tk('Find and replace'), '⌘F'], [tk('Rulers'), '⇧R'],
  [tk('Duplicate while dragging'), tk('⌥ + drag')], [tk('Disable snapping while dragging'), tk('⌘ + drag')],
];

export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  useEditorState();
  return (
    <Modal title={t('Keyboard shortcuts')} onClose={onClose} size="md">
      <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: '8px 16px' }}>
        {SHORTCUTS.map(([a, k]) => (
          <div key={a} style={{ display: 'contents' }}>
            <span>{t(a)}</span>
            {/* Key glyphs are not translated; only the few worded ones (e.g. 'Space + drag') have catalog entries. */}
            <span className="kbd">{t(k)}</span>
          </div>
        ))}
      </div>
    </Modal>
  );
}
