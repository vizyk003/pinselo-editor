import { formatDims, type Unit } from '@oc/engine';
import {
  BookmarkPlus, Copy, Download, ExternalLink, Folder, FolderInput, Info, LayoutTemplate, Link2, MoreHorizontal, Pencil, RotateCcw, Share2, Star, Trash2,
} from 'lucide-react';
import { useEffect, useState, type MouseEvent, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { api, thumbUrl, type DesignSummary, type Folder as FolderT } from '../../api';
import { useAuth } from '../../App';
import { FolderPicker } from '../../components/FolderPicker';
import { EmptyState, MenuItem, Modal, PopoverButton, timeAgo, useToast } from '../../components/ui';
import { fmtDate, t, tk, tn } from '../../i18n';
import { designKey, folderPath, type ViewMode } from './projectsModel';
import './projects.css';

/** Design types (surfaces) as the cards' badge and the Details dialog name them. */
const SURFACE_LABELS: Record<string, string> = {
  graphic: tk('Graphic'), presentation: tk('Presentation'), video: tk('Video'), doc: tk('Doc'), sheet: tk('Sheet'),
  whiteboard: tk('Whiteboard'), print: tk('Print'), website: tk('Website'), email: tk('Email'),
};
export const surfaceLabel = (s: string) => t(SURFACE_LABELS[s] ?? s);

/** Checkbox selection of a list (Projects bulk select, g17). Keys are designKey(id) / folderKey(id). */
export interface Selection { active: boolean; has: (key: string) => boolean; toggle: (key: string, shift: boolean) => void }

/** Column headings of the list view (folders and designs share the columns). */
export function ListHead({ dateLabel }: { dateLabel?: string }) {
  return (
    <div className="pj-row head" aria-hidden>
      <span /><span /><span>{t('Name')}</span><span className="pj-col">{t('Type')}</span><span className="pj-col">{t('Owner')}</span>
      <span className="pj-col">{dateLabel ?? t('Date modified')}</span><span />
    </div>
  );
}

export function DesignGrid({ designs, reload, folders, empty, trash, view = 'grid', selection }: {
  designs: DesignSummary[] | null; reload: () => void; folders: FolderT[];
  /** Shown when there are no designs: a line of text (in an EmptyState) or a ready element; '' shows nothing. */
  empty: ReactNode; trash?: boolean;
  /** Cards in a grid, rows in a list, or one scrolling row (the Recents carousel). */
  view?: ViewMode | 'row';
  selection?: Selection;
}) {
  if (!designs) return <div className="empty">{t('Loading…')}</div>;
  if (!designs.length) return typeof empty === 'string' ? (empty ? <EmptyState icon={<LayoutTemplate size={22} />}>{empty}</EmptyState> : null) : <>{empty}</>;
  const cards = designs.map((d) => <DesignCard key={d.id} d={d} reload={reload} folders={folders} trash={trash} list={view === 'list'} selection={selection} />);
  if (view === 'list') return <div className="pj-list"><ListHead />{cards}</div>;
  return <div className={view === 'row' ? 'pj-track' : 'grid'}>{cards}</div>;
}

function DesignCard({ d, reload, folders, trash, list, selection }: { d: DesignSummary; reload: () => void; folders: FolderT[]; trash?: boolean; list?: boolean; selection?: Selection }) {
  const nav = useNavigate();
  const toast = useToast();
  const { user } = useAuth();
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(d.title);
  const [imgOk, setImgOk] = useState(true);
  const [starred, setStarred] = useState(d.starred);
  const [dialog, setDialog] = useState<'move' | 'details' | null>(null);
  useEffect(() => setStarred(d.starred), [d.starred]);
  const mine = d.owner_id === user?.id;
  const intents = d.surface !== 'doc' && d.surface !== 'sheet';
  const key = designKey(d.id);
  const selected = selection?.has(key) ?? false;
  const open = () => !trash && nav(`/design/${d.id}`);
  // While items are selected (or with Shift/Cmd/Ctrl), a click selects instead of opening.
  const onClick = (e: MouseEvent) => {
    if (selection && (selection.active || e.shiftKey || e.metaKey || e.ctrlKey)) selection.toggle(key, e.shiftKey);
    else open();
  };
  const toggleStar = async () => {
    setStarred(!starred);
    try {
      await api.updateDesign(d.id, { starred: !starred });
      reload();
    } catch (e) {
      setStarred(starred);
      toast((e as Error).message);
    }
  };
  const moveTo = async (folderId: string | null) => {
    setDialog(null);
    if (folderId === d.folder_id) return;
    try {
      await api.updateDesign(d.id, { folderId });
      reload();
      toast(t('Moved to {folder}', { folder: folderId ? folders.find((f) => f.id === folderId)?.name ?? t('Folder') : t('Projects') }));
    } catch (e) {
      toast((e as Error).message);
    }
  };
  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(`${location.origin}/design/${d.id}`);
      toast(t('Link copied'));
    } catch {
      toast(t('Could not copy the link'));
    }
  };

  const check = selection && (
    <input type="checkbox" className="pj-check" checked={selected} aria-label={d.title}
      onChange={() => {}} onClick={(e) => { e.stopPropagation(); selection.toggle(key, e.shiftKey); }} />
  );
  const thumb = imgOk ? <img src={thumbUrl(d.id, d.updated_at)} alt="" loading="lazy" onError={() => setImgOk(false)} /> : <LayoutTemplate size={list ? 18 : 32} className="faint" />;
  const name = renaming ? (
    <input
      className="input" style={{ height: 28 }} value={title} autoFocus onChange={(e) => setTitle(e.target.value)} onClick={(e) => e.stopPropagation()}
      onBlur={async () => {
        setRenaming(false);
        if (title.trim() && title !== d.title) {
          await api.updateDesign(d.id, { title: title.trim() });
          reload();
        }
      }}
      onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
    />
  ) : list && !trash ? (
    // In the list, the name is the row's keyboard entry point (Enter and Space); the row is only a bigger mouse target,
    // so the checkbox, star and menu stay separate controls and the row's other columns are read as they are.
    <button type="button" className="pj-open pj-title pj-ellipsis" title={d.title} onClick={(e) => { e.stopPropagation(); onClick(e); }}>{d.title}</button>
  ) : <div className={list ? 'pj-title pj-ellipsis' : 't'} title={d.title}>{d.title}</div>;
  const actions = (
    <div className="pj-actions" onClick={(e) => e.stopPropagation()}>
      {!trash && (
        <button type="button" className={`btn sm icon ghost pj-star ${starred ? 'on' : ''}`} aria-pressed={starred} title={starred ? t('Unstar') : t('Star')} aria-label={starred ? t('Unstar') : t('Star')} onClick={() => void toggleStar()}>
          <Star size={16} fill={starred ? 'currentColor' : 'none'} />
        </button>
      )}
      <PopoverButton className="btn sm icon" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={(close) => (
        <div className="menu" role="menu">
          {trash ? (
            <>
              <MenuItem icon={<RotateCcw size={16} />} label={t('Restore')} onClick={async () => { close(); await api.restoreDesign(d.id); reload(); toast(t('Design restored')); }} />
              <MenuItem icon={<Info size={16} />} label={t('Details')} onClick={() => { close(); setDialog('details'); }} />
              <MenuItem icon={<Trash2 size={16} />} label={t('Delete forever')} danger onClick={async () => { close(); await api.trashDesign(d.id, true); reload(); }} />
            </>
          ) : (
            <>
              <MenuItem icon={<Pencil size={16} />} label={t('Rename')} onClick={() => { close(); setRenaming(true); }} disabled={!mine} />
              <MenuItem icon={<Star size={16} />} label={starred ? t('Unstar') : t('Star')} onClick={() => { close(); void toggleStar(); }} />
              <MenuItem icon={<Copy size={16} />} label={t('Make a copy')} onClick={async () => { close(); await api.copyDesign(d.id); reload(); toast(t('Copy created')); }} />
              <MenuItem icon={<Share2 size={16} />} label={t('Open & share')} onClick={() => { close(); nav(`/design/${d.id}?share=1`); }} />
              <MenuItem icon={<ExternalLink size={16} />} label={t('Open in new tab')} onClick={() => { close(); window.open(`/design/${d.id}`, '_blank', 'noopener'); }} />
              <MenuItem icon={<Link2 size={16} />} label={t('Copy link')} onClick={() => { close(); void copyLink(); }} />
              {/* The design editor opens its Download / Publish as template dialog from these one-shot intents (docs and sheets have their own editors without them). */}
              {intents && <MenuItem icon={<Download size={16} />} label={t('Download')} onClick={() => { close(); nav(`/design/${d.id}?download=1`); }} />}
              <MenuItem icon={<Info size={16} />} label={t('Details')} onClick={() => { close(); setDialog('details'); }} />
              {mine && intents && <MenuItem icon={<BookmarkPlus size={16} />} label={t('Save as template')} onClick={() => { close(); nav(`/design/${d.id}?publishTemplate=1`); }} />}
              {mine && <div className="hsep" />}
              {mine && <MenuItem icon={<FolderInput size={16} />} label={t('Move to folder')} onClick={() => { close(); setDialog('move'); }} />}
              {mine && <MenuItem icon={<Trash2 size={16} />} label={t('Move to trash')} danger onClick={async () => { close(); await api.trashDesign(d.id); reload(); toast(t('Moved to trash')); }} />}
            </>
          )}
        </div>
      )} />
    </div>
  );
  const dialogs = (
    <>
      {dialog === 'move' && <FolderPicker value={d.folder_id} onClose={() => setDialog(null)} onPick={(f) => void moveTo(f)} />}
      {dialog === 'details' && <DesignDetails d={d} folders={folders} mine={mine} onClose={() => setDialog(null)} />}
    </>
  );

  if (list) {
    // Dialogs render outside the row: events from a portal still bubble to its React parent.
    return (
      <>
      <div className={`pj-row ${selected ? 'selected' : ''}`} onClick={onClick}>
        {check ?? <span />}
        <div className="pj-thumb">{thumb}</div>
        <div style={{ minWidth: 0 }}>
          {name}
          <div className="pj-sub pj-ellipsis">{formatDims(d.width, d.height, d.unit as Unit)} · {tn(d.page_count, '{n} page', '{n} pages')}</div>
        </div>
        <span className="pj-col pj-ellipsis">{surfaceLabel(d.surface)}</span>
        <span className="pj-col pj-ellipsis">{mine ? t('You') : d.owner_name}</span>
        <span className="pj-col pj-ellipsis">{timeAgo(d.updated_at)}</span>
        {actions}
      </div>
      {dialogs}
      </>
    );
  }
  return (
    <div className={`card pj-card ${selected ? 'selected' : ''}`}>
      <div className="thumb" onClick={onClick} role="button" tabIndex={0} aria-label={t('Open {title}', { title: d.title })}
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget || (e.key !== 'Enter' && e.key !== ' ')) return;
          e.preventDefault();
          open();
        }}>
        {thumb}
        {d.surface !== 'graphic' && <span className="pj-badge">{surfaceLabel(d.surface)}</span>}
      </div>
      {check}
      <div className="meta">
        <div className="grow" style={{ minWidth: 0 }}>
          {name}
          <div className="s">
            {formatDims(d.width, d.height, d.unit as Unit)} · {tn(d.page_count, '{n} page', '{n} pages')} · {timeAgo(d.updated_at)}
            {!mine && ` · ${t('by {owner}', { owner: d.owner_name })}`}
          </div>
        </div>
      </div>
      {actions}
      {dialogs}
    </div>
  );
}

/** Details of a design (g18): type, size, pages, owner, dates and where it's filed. */
function DesignDetails({ d, folders, mine, onClose }: { d: DesignSummary; folders: FolderT[]; mine: boolean; onClose: () => void }) {
  const when = (iso: string) => fmtDate(iso, { dateStyle: 'medium', timeStyle: 'short' });
  // A folder the user can't see (someone else's) shows as a dash.
  const path = d.folder_id ? folderPath(folders, d.folder_id).map((f) => f.name).join(' / ') || '—' : t('Projects');
  const rows: Array<[string, ReactNode]> = [
    [t('Type'), surfaceLabel(d.surface)],
    [t('Size'), formatDims(d.width, d.height, d.unit as Unit)],
    [t('Pages'), d.page_count],
    [t('Owner'), mine ? t('You') : d.owner_name],
    [t('Created'), when(d.created_at)],
    [t('Date modified'), when(d.updated_at)],
    [t('Folder'), path],
  ];
  return (
    <Modal title={t('Details')} onClose={onClose} size="sm">
      <div className="row" style={{ gap: 12 }}>
        <div className="pj-thumb"><img src={thumbUrl(d.id, d.updated_at)} alt="" /></div>
        <b className="pj-ellipsis">{d.title}</b>
      </div>
      <dl className="pj-details">
        {rows.map(([k, v]) => <div key={k} style={{ display: 'contents' }}><dt>{k}</dt><dd>{v}</dd></div>)}
      </dl>
    </Modal>
  );
}
