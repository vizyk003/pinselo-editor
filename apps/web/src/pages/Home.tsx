import { BRAND } from '@oc/config';
import type { PresetCategoryId } from '@oc/engine';
import {
  CalendarDays, CheckSquare, Folder, FolderPlus, HelpCircle, Home as HomeIcon, LogOut, Menu, Pencil, Plus,
  Settings as SettingsIcon, Shield, Star, Trash2, UploadCloud, Users, LayoutTemplate,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, NavLink, Route, Routes, useLocation, useNavigate, useParams } from 'react-router';
import { PlannerPage } from '../social/Planner';
import { api, type DesignSummary, type Folder as FolderT, type Task } from '../api';
import { projectsApi, type ProjectFolder } from '../api/projects';
import { useAuth } from '../App';
import { signOut } from '../auth/session';
import { NotificationsBell } from '../components/Notifications';
import { Avatar, EmptyState, interp, PageHeader, useToast } from '../components/ui';
import { Admin } from './Admin';
import { Settings } from './Settings';
import { CreateTeamDialog, TeamPage, useWorkspaces } from './Team';
import { fmtDate, t } from '../i18n';
import { CreateDialog } from './home/CreateDialog';
import { Dashboard } from './home/Dashboard';
import { StartUploadDialog } from './home/StartFromUpload';
import { DesignList, ProjectsPage } from './home/Projects';
import { childFolders } from './home/projectsModel';
import { parseCreateIntent } from './home/quickTiles';
import { SearchResultsPage } from './home/SearchResults';
import { TemplatesPage } from './home/Templates';
import { TrashPage } from './home/Trash';
import { UploadsPage } from './home/Uploads';

// Screens split out into ./home/; re-exported so existing imports keep working.
export { CreateDialog } from './home/CreateDialog';
export { QUICK } from './home/Dashboard';
export { TemplateCard } from './home/Templates';
export { DesignList } from './home/Projects';

export function Home() {
  const { user, setUser } = useAuth();
  const nav = useNavigate();
  const toast = useToast();
  // Waits only for the Pinselo session to end; Supabase's sign-out finishes in the background (review C3).
  const logOut = () => signOut().then(() => {
    setUser(null);
    nav('/', { replace: true });
  }, () => toast(t('Couldn’t sign out. Check your connection and try again.')));
  const [creating, setCreating] = useState(false);
  /** Category the Create dialog opens on (a tile's 'See all sizes', Custom size). */
  const [createCat, setCreateCat] = useState<PresetCategoryId | undefined>();
  const openCreate = (cat?: PresetCategoryId) => { setCreateCat(cat); setCreating(true); };
  /** Upload / Photo editor dialog, opened by the `?create=upload|photo` intent (g15). */
  const [uploading, setUploading] = useState<'upload' | 'photo' | null>(null);
  const [newTeam, setNewTeam] = useState(false);
  const teams = useWorkspaces();
  const [folders, setFolders] = useState<ProjectFolder[]>([]);
  const reloadFolders = useCallback(() => projectsApi.folders().then((r) => setFolders(r.folders)).catch(() => {}), []);
  /** The ?create= / ?template= shortcut already acted on by this Home (effects run twice in development). */
  const shortcutHandled = useRef<string | null>(null);
  /** Narrow screens (≤900 px): the sidebar opens as a drawer from the menu button. */
  const [navOpen, setNavOpen] = useState(false);
  const menuBtn = useRef<HTMLButtonElement>(null);
  const sideRef = useRef<HTMLElement>(null);
  const { pathname } = useLocation();
  useEffect(() => setNavOpen(false), [pathname]);
  useEffect(() => {
    if (!navOpen) return;
    sideRef.current?.querySelector<HTMLElement>('a, button')?.focus();
    const k = (e: KeyboardEvent) => { if (e.key === 'Escape') { setNavOpen(false); menuBtn.current?.focus(); } };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [navOpen]);
  useEffect(() => {
    void reloadFolders();
  }, [reloadFolders]);
  // App shortcuts (PWA manifest / home-screen long-press): /?create=<size preset>.
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const create = params.get('create');
    const template = params.get('template');
    const key = create ? `c:${create}` : template ? `t:${template}` : null;
    if (key === shortcutHandled.current) return;
    shortcutHandled.current = key;
    // Also the actions resumed after signing in from the public home (AuthGate).
    const intent = parseCreateIntent(create);
    // More / See all sizes / Custom size: the Create dialog on Suggestions or on a category.
    if (intent?.kind === 'dialog') { openCreate(intent.category); nav('/', { replace: true }); }
    // Upload and Photo editor tiles of the public home: a file picker needs a click, so offer the upload dialog.
    else if (intent?.kind === 'upload' || intent?.kind === 'photo') { setUploading(intent.kind); nav('/', { replace: true }); }
    else if (intent?.kind === 'preset') void api.createDesign({ preset: intent.preset }).then((r) => nav(`/design/${r.id}`, { replace: true }));
    else if (template && /^[\w-]{1,80}$/.test(template)) {
      void api.createDesign({ templateId: template })
        .then((r) => nav(`/design/${r.id}`, { replace: true }))
        .catch(() => nav('/templates', { replace: true }));
    }
  }, [nav]);

  return (
    <div className="home">
      <div className="home-mobilebar">
        <button ref={menuBtn} className="btn ghost icon" aria-label={t('Open menu')} title={t('Open menu')} aria-expanded={navOpen} aria-controls="home-side" onClick={() => setNavOpen(true)}>
          <Menu size={20} />
        </button>
        <Link to="/" className="logo"><img src="/favicon.svg" alt="" /><span>{BRAND.name}</span></Link>
        <div className="grow" />
        <button className="btn primary sm" onClick={() => openCreate()}><Plus size={16} /> {t('Create a design')}</button>
      </div>
      {navOpen && <div className="home-scrim" onClick={() => setNavOpen(false)} aria-hidden />}
      <nav ref={sideRef} id="home-side" className={`side${navOpen ? ' open' : ''}`} aria-label={t('Main')}
        onClick={(e) => (e.target as HTMLElement).closest('a') && setNavOpen(false)}>
        <div className="logo" style={{ padding: '4px 4px 14px 12px' }}>
          <Link to="/" className="logo"><img src="/favicon.svg" alt="" /><span>{BRAND.name}</span></Link>
          <NotificationsBell />
        </div>
        <button className="btn primary" style={{ margin: '0 4px 12px' }} onClick={() => { setNavOpen(false); openCreate(); }}>
          <Plus size={16} /> {t('Create a design')}
        </button>
        <NavLink to="/" end><HomeIcon size={18} /> {t('Home')}</NavLink>
        <NavLink to="/projects"><Folder size={18} /> {t('Projects')}</NavLink>
        <NavLink to="/uploads"><UploadCloud size={18} /> {t('Uploads')}</NavLink>
        <NavLink to="/templates"><LayoutTemplate size={18} /> {t('Templates')}</NavLink>
        <NavLink to="/starred"><Star size={18} /> {t('Starred')}</NavLink>
        <NavLink to="/shared"><Users size={18} /> {t('Shared with you')}</NavLink>
        <NavLink to="/tasks"><CheckSquare size={18} /> {t('Tasks')}</NavLink>
        <NavLink to="/planner"><CalendarDays size={18} /> {t('Content Planner')}</NavLink>
        <NavLink to="/trash"><Trash2 size={18} /> {t('Trash')}</NavLink>
        {folders.length > 0 && <div className="section-label">{t('Folders')}</div>}
        {/* Top-level folders only: subfolders are reached through Projects (g17). */}
        {childFolders(folders, null).map((f) => (
          <NavLink key={f.id} to={`/projects/${f.id}`}><Folder size={16} /> {f.name}</NavLink>
        ))}
        <div className="section-label">{t('Teams')}</div>
        {teams.list.filter((w) => !w.personal).map((w) => (
          <NavLink key={w.id} to={`/team/${w.id}`}><Users size={16} /> {w.name}</NavLink>
        ))}
        <button className="navlink" onClick={() => { setNavOpen(false); setNewTeam(true); }}><Plus size={16} /> {t('Create a team')}</button>
        <div className="grow" />
        <NavLink to="/help"><HelpCircle size={18} /> {t('Help')}</NavLink>
        {user!.staff && <NavLink to="/admin"><Shield size={18} /> {t('Admin')}</NavLink>}
        <NavLink to="/settings"><SettingsIcon size={18} /> {t('Settings')}</NavLink>
        <div className="row" style={{ padding: '8px 8px 0' }}>
          <Avatar name={user!.name} color={user!.color} />
          <div className="grow" style={{ minWidth: 0 }}>
            <div className="ellipsis" style={{ fontWeight: 600 }} title={user!.name}>{user!.name}</div>
            <div className="faint ellipsis" style={{ fontSize: 12 }} title={user!.email}>{user!.email}</div>
          </div>
          <button className="btn ghost icon sm" title={t('Sign out')} aria-label={t('Sign out')} onClick={() => void logOut()}>
            <LogOut size={16} />
          </button>
        </div>
      </nav>
      <main>
        {user && user.verified === false && (
          <div className="banner home-verify">
            <span className="grow">{interp(t('Confirm your email — we sent a link to {email}.'), { email: <b>{user.email}</b> })}</span>
            <NavLink className="linkish" to="/settings/account">{t('Resend')}</NavLink>
          </div>
        )}
        <Routes>
          <Route path="/" element={<Dashboard onCreate={openCreate} folders={folders} />} />
          <Route path="/projects" element={<ProjectsPage folders={folders} reload={reloadFolders} />} />
          <Route path="/projects/:folderId" element={<ProjectsPage folders={folders} reload={reloadFolders} />} />
          <Route path="/uploads" element={<UploadsPage />} />
          <Route path="/templates" element={<TemplatesPage />} />
          <Route path="/search" element={<SearchResultsPage folders={folders} />} />
          <Route path="/tasks" element={<TasksPage />} />
          <Route path="/planner" element={<PlannerPage />} />
          <Route path="/starred" element={<DesignList title={t('Starred')} filter="starred" folders={folders} reloadFolders={reloadFolders} />} />
          <Route path="/shared" element={<DesignList title={t('Shared with you')} filter="shared" folders={folders} />} />
          <Route path="/team/:id" element={<TeamPage folders={folders} workspaces={teams.list} reloadWorkspaces={teams.reload} />} />
          <Route path="/settings/*" element={<Settings />} />
          <Route path="/admin/*" element={<Admin />} />
          <Route path="/trash" element={<TrashPage folders={folders} />} />
        </Routes>
      </main>
      {creating && <CreateDialog onClose={() => setCreating(false)} initialCategory={createCat} />}
      {uploading && <StartUploadDialog mode={uploading} onClose={() => setUploading(null)} />}
      {newTeam && <CreateTeamDialog onClose={() => setNewTeam(false)} onCreated={(id) => { setNewTeam(false); void teams.reload(); nav(`/team/${id}`); }} />}
    </div>
  );
}
/** Comments assigned to me across designs (CO-030). */
function TasksPage() {
  const nav = useNavigate();
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const load = useCallback(() => api.tasks().then((r) => setTasks(r.tasks)), []);
  useEffect(() => { void load(); }, [load]);
  const today = new Date().toISOString().slice(0, 10);
  return (
    <>
    <PageHeader title={t('Tasks')} />
    <div className="content">
      {!tasks ? <div className="spinner" /> : !tasks.length ? <EmptyState icon={<CheckSquare size={22} />}>{t('No open tasks. Tasks are comments someone assigned to you.')}</EmptyState> : (
        <div className="list">
          {tasks.map((x) => (
            <div className="item" key={x.id}>
              <input type="checkbox" aria-label={t('Mark done')} onChange={async () => { await api.resolveComment(x.id, true); void load(); }} />
              <div className="main grow" style={{ cursor: 'pointer' }} onClick={() => nav(`/design/${x.design_id}?comments=1`)}>
                <div style={{ whiteSpace: 'pre-wrap' }}>{x.body}</div>
                <div className="sub">{x.design_title} · {x.author}{x.due_on ? ` · ${t('Due {date}', { date: fmtDate(`${x.due_on}T12:00:00`) })}` : ''}{x.due_on && x.due_on < today ? ` · ${t('Overdue')}` : ''}</div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
    </>
  );
}
