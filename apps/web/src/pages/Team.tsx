/** Team workspaces (CO-060/061): team designs, members and roles, email invites, member groups (g149). */
import { Pencil, Plus, ShieldCheck, Trash2, UserPlus, Users } from 'lucide-react';
import { brandHub } from '../api';
import { BrandControlsDialog } from './BrandControls';
import { useCallback, useEffect, useId, useState } from 'react';
import { useNavigate, useParams } from 'react-router';
import { ApiError, teamApi, type Folder, type Invite, type Member, type TeamRole, type Workspace } from '../api';
import { groupsApi, type TeamGroup } from '../api/sharing';
import { useAuth } from '../App';
import { tabPanelProps, tabProps } from '../components/tabs';
import { Avatar, Modal, timeAgo, useToast } from '../components/ui';
import { CreateDialog, DesignList } from './Home';
import { fmtDate, fmtList, t, tk, tn } from '../i18n';

const ROLE_LABEL: Record<TeamRole, string> = { owner: tk('Owner'), admin: tk('Admin'), designer: tk('Designer'), member: tk('Member') };
const ROLE_HELP: Record<TeamRole, string> = {
  owner: tk('Full control, including deleting the team.'),
  admin: tk('Manages members and every team design.'),
  designer: tk('Creates and edits team designs.'),
  member: tk('Views and comments on team designs.'),
};
const RANK: Record<TeamRole, number> = { member: 1, designer: 2, admin: 3, owner: 4 };

export function useWorkspaces() {
  const [list, setList] = useState<Workspace[]>([]);
  const reload = useCallback(() => teamApi.list().then((r) => setList(r.workspaces)).catch(() => {}), []);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { list, reload };
}

export function CreateTeamDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (id: string) => void }) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('Create a team')} onClose={onClose} size="sm">
      <p className="muted" style={{ margin: 0 }}>{t('Teams share designs, brand kits and folders. You can invite people right after.')}</p>
      <form className="col" onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void teamApi.create(name.trim()).then((r) => onCreated(r.id)).finally(() => setBusy(false));
      }}>
        <label className="field">{t('Team name')}<input className="input" value={name} onChange={(e) => setName(e.target.value)} autoFocus required maxLength={80} placeholder={t('Marketing')} /></label>
        <button className="btn primary" disabled={busy || !name.trim()}>{t('Create team')}</button>
      </form>
    </Modal>
  );
}

export function TeamPage({ folders, workspaces, reloadWorkspaces }: { folders: Folder[]; workspaces: Workspace[]; reloadWorkspaces: () => void }) {
  const { id } = useParams() as { id: string };
  const ws = workspaces.find((w) => w.id === id);
  const [members, setMembers] = useState<false | 'people' | 'groups'>(false);
  const [creating, setCreating] = useState(false);
  const [controls, setControls] = useState(false);
  const [requireTemplate, setRequireTemplate] = useState(false);
  useEffect(() => {
    if (ws) void brandHub.controls(ws.id).then((r) => setRequireTemplate(Boolean(r.controls.requireTemplate))).catch(() => {});
  }, [ws?.id, controls]);
  if (!ws) return <div className="empty">{workspaces.length ? t('You’re not a member of this team.') : t('Loading…')}</div>;
  // Members start team designs from brand templates; designers too when the team requires it (A8).
  const canCreate = true;
  const templatesOnly = ws.role === 'member' || (ws.role === 'designer' && requireTemplate);
  return (
    <>
      <DesignList
        title={ws.name}
        filter="team"
        params={{ workspace: id }}
        folders={folders}
        actions={
          <>
            <button className="btn" onClick={() => setMembers('people')}><Users size={16} /> {tn(ws.members, '{n} member', '{n} members')}</button>
            <button className="btn" onClick={() => setMembers('groups')}>{t('Groups')}</button>
            {RANK[ws.role] >= RANK.admin && <button className="btn" onClick={() => setControls(true)}><ShieldCheck size={16} /> {t('Brand controls')}</button>}
            {canCreate && <button className="btn primary" onClick={() => setCreating(true)}><Plus size={16} /> {t('Create design')}</button>}
          </>
        }
      />
      {creating && <CreateDialog onClose={() => setCreating(false)} workspaceId={id} templatesOnly={templatesOnly} />}
      {controls && <BrandControlsDialog workspaceId={id} onClose={() => setControls(false)} />}
      {members && <MembersDialog ws={ws} initialTab={members} onClose={() => { setMembers(false); reloadWorkspaces(); }} />}
    </>
  );
}

/** Members as listed by the API, with their groups in this team (the Groups column). */
type MemberRow = Member & { groups?: Array<{ id: string; name: string }> };

const MEMBER_TABS = ['people', 'groups'] as const;

function MembersDialog({ ws, initialTab = 'people', onClose }: { ws: Workspace; initialTab?: 'people' | 'groups'; onClose: () => void }) {
  const { user } = useAuth();
  const toast = useToast();
  const nav = useNavigate();
  const [tab, setTab] = useState(initialTab);
  const tabsId = useId();
  const [data, setData] = useState<{ members: MemberRow[]; invites: Invite[] } | null>(null);
  const [emails, setEmails] = useState('');
  const [role, setRole] = useState<Exclude<TeamRole, 'owner'>>('designer');
  const [err, setErr] = useState('');
  const load = useCallback(() => teamApi.members(ws.id).then(setData), [ws.id]);
  useEffect(() => {
    void load();
  }, [load]);
  const manage = RANK[ws.role] >= RANK.admin;
  const act = (p: Promise<unknown>, ok?: string) =>
    p.then(() => { if (ok) toast(ok); return load(); }).catch((e) => setErr(e instanceof ApiError ? e.message : t('That didn’t work.')));

  const invite = () => {
    const list = emails.split(/[\s,;]+/).map((s) => s.trim()).filter(Boolean);
    if (!list.length) return;
    setErr('');
    void act(teamApi.invite(ws.id, list, role).then(() => setEmails('')), tn(list.length, 'Invited {n} person', 'Invited {n} people'));
  };

  return (
    <Modal title={t('{team} · members', { team: ws.name })} onClose={onClose} size="lg">
      <div className="tabs" role="tablist" aria-label={t('Members')}>
        <button {...tabProps(tabsId, MEMBER_TABS, tab, setTab, 'people')} className={tab === 'people' ? 'active' : ''}>{t('People')}</button>
        <button {...tabProps(tabsId, MEMBER_TABS, tab, setTab, 'groups')} className={tab === 'groups' ? 'active' : ''}>{t('Groups')}</button>
      </div>
      <div {...tabPanelProps(tabsId, tab)} className="col" style={{ gap: 12 }}>
      {tab === 'groups' && <GroupsSection ws={ws} members={data?.members ?? []} manage={manage} onChanged={() => void load()} />}
      {tab === 'people' && <>
      {manage && (
        <div className="col">
          <label className="field">{t('Invite by email')}</label>
          <div className="row">
            <input className="input grow" value={emails} onChange={(e) => setEmails(e.target.value)} placeholder={t('name@example.com, another@example.com')} onKeyDown={(e) => e.key === 'Enter' && invite()} />
            <select className="input" style={{ width: 130 }} value={role} onChange={(e) => setRole(e.target.value as typeof role)} title={t(ROLE_HELP[role])}>
              {(['admin', 'designer', 'member'] as const).map((r) => <option key={r} value={r}>{t(ROLE_LABEL[r])}</option>)}
            </select>
            <button className="btn primary" onClick={invite} disabled={!emails.trim()}><UserPlus size={16} /> {t('Invite')}</button>
          </div>
          <span className="faint" style={{ fontSize: 12 }}>{t(ROLE_HELP[role])}</span>
        </div>
      )}
      {err && <div className="form-error">{err}</div>}
      <div className="list">
        {data?.members.map((m) => {
          const self = m.id === user?.id;
          const canEdit = manage && !self && RANK[m.role] <= RANK[ws.role];
          return (
            <div className="item" key={m.id}>
              <Avatar name={m.name} color={m.color} />
              <div className="main">
                <div>{m.name}{self && <span className="faint"> {t('(you)')}</span>}</div>
                <div className="sub">{m.email} · {t('joined {when}', { when: timeAgo(m.created_at) })}</div>
              </div>
              {!!m.groups?.length && (
                <div className="row" style={{ gap: 4, flexWrap: 'wrap', justifyContent: 'flex-end', maxWidth: 180 }} aria-label={t('Groups')}>
                  {m.groups.map((g) => <span key={g.id} className="pill">{g.name}</span>)}
                </div>
              )}
              {canEdit ? (
                <select className="input" style={{ width: 120 }} value={m.role} onChange={(e) => void act(teamApi.setRole(ws.id, m.id, e.target.value as TeamRole), t('Role updated'))}>
                  {(Object.keys(ROLE_LABEL) as TeamRole[]).filter((r) => r !== 'owner' || ws.role === 'owner').map((r) => <option key={r} value={r}>{t(ROLE_LABEL[r])}</option>)}
                </select>
              ) : (
                <span className="pill">{t(ROLE_LABEL[m.role])}</span>
              )}
              {canEdit && <button className="btn ghost sm danger" onClick={() => confirm(t('Remove {name} from {team}?', { name: m.name, team: ws.name })) && void act(teamApi.remove(ws.id, m.id), t('Removed'))}>{t('Remove')}</button>}
              {self && (
                <button className="btn ghost sm danger" onClick={() => {
                  if (!confirm(t('Leave {team}? You’ll lose access to its designs.', { team: ws.name }))) return;
                  teamApi.remove(ws.id, m.id).then(() => nav('/')).catch((e) => setErr(e instanceof ApiError ? e.message : t('That didn’t work.')));
                }}>{t('Leave')}</button>
              )}
            </div>
          );
        })}
      </div>
      {!!data?.invites.length && (
        <>
          <div className="section-label" style={{ padding: 0 }}>{t('Pending invites')}</div>
          <div className="list">
            {data.invites.map((i) => (
              <div className="item" key={i.id}>
                <div className="main"><div>{i.email}</div><div className="sub">{t(ROLE_LABEL[i.role])} · {t('expires {date}', { date: fmtDate(i.expires_at) })}</div></div>
                {manage && <button className="btn ghost sm" onClick={() => void act(teamApi.cancelInvite(ws.id, i.id), t('Invite cancelled'))}>{t('Cancel')}</button>}
              </div>
            ))}
          </div>
        </>
      )}
      </>}
      </div>
    </Modal>
  );
}

const GROUP_ERRORS: Record<string, string> = {
  group_exists: tk('A group with that name already exists.'),
  not_member: tk('Only members of this team can join its groups.'),
  forbidden: tk('Only owners and admins can manage groups.'),
};

/** Member groups (g149): named sets of teammates you can share designs with in one go. */
function GroupsSection({ ws, members, manage, onChanged }: { ws: Workspace; members: MemberRow[]; manage: boolean; onChanged: () => void }) {
  const toast = useToast();
  const [groups, setGroups] = useState<TeamGroup[] | null>(null);
  const [name, setName] = useState('');
  const [openId, setOpenId] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(null);
  const [err, setErr] = useState('');
  const load = useCallback(() => groupsApi.list(ws.id).then((r) => setGroups(r.groups)).catch(() => setGroups([])), [ws.id]);
  useEffect(() => {
    void load();
  }, [load]);
  const byId = new Map(members.map((m) => [m.id, m]));
  const act = (p: Promise<unknown>, ok?: string) => {
    setErr('');
    return p.then(() => {
      if (ok) toast(ok);
      onChanged();
      return load();
    }).catch((e) => setErr(e instanceof ApiError && GROUP_ERRORS[e.code] ? t(GROUP_ERRORS[e.code]!) : t('That didn’t work.')));
  };
  return (
    <div className="col">
      <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('Groups let you share a design with several teammates at once. Everyone in the team can see the groups and share with them.')}</p>
      {manage && (
        <form className="row" onSubmit={(e) => {
          e.preventDefault();
          if (!name.trim()) return;
          void act(groupsApi.create(ws.id, name.trim()).then((r) => { setName(''); setOpenId(r.id); }), t('Group created'));
        }}>
          <input className="input grow" value={name} onChange={(e) => setName(e.target.value)} maxLength={60} placeholder={t('New group name, e.g. Marketing')} aria-label={t('Group name')} />
          <button className="btn primary" disabled={!name.trim()}><Plus size={16} /> {t('Create group')}</button>
        </form>
      )}
      {err && <div className="form-error">{err}</div>}
      {groups && !groups.length && <div className="faint" style={{ fontSize: 13 }}>{manage ? t('No groups yet. Create one, then add people to it.') : t('This team has no groups yet.')}</div>}
      <div className="list">
        {groups?.map((g) => {
          const people = g.memberIds.map((id) => byId.get(id)).filter((m): m is MemberRow => Boolean(m));
          const open = openId === g.id;
          return (
            <div key={g.id} className="col" style={{ gap: 8, padding: '10px 0', borderTop: '1px solid var(--border)' }}>
              <div className="row">
                <span aria-hidden style={{ flex: 'none', width: 30, height: 30, borderRadius: '50%', display: 'grid', placeItems: 'center', background: 'var(--surface-3)' }}><Users size={16} /></span>
                <div className="grow" style={{ minWidth: 0 }}>
                  {editing?.id === g.id ? (
                    <form className="row" style={{ gap: 6 }} onSubmit={(e) => {
                      e.preventDefault();
                      if (!editing.name.trim()) return;
                      void act(groupsApi.rename(ws.id, g.id, editing.name.trim()).then(() => setEditing(null)), t('Group renamed'));
                    }}>
                      <input className="input" autoFocus value={editing.name} maxLength={60} onChange={(e) => setEditing({ id: g.id, name: e.target.value })} aria-label={t('Group name')} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setEditing(null); } }} />
                      <button className="btn sm primary" disabled={!editing.name.trim()}>{t('Save')}</button>
                      <button type="button" className="btn sm ghost" onClick={() => setEditing(null)}>{t('Cancel')}</button>
                    </form>
                  ) : (
                    <>
                      <div><b>{g.name}</b></div>
                      <div className="faint" style={{ fontSize: 12 }}>{tn(people.length, '{n} member', '{n} members')}{people.length ? ` · ${fmtList(people.length > 4 ? [...people.slice(0, 3).map((p) => p.name), tn(people.length - 3, '{n} other', '{n} others')] : people.map((p) => p.name))}` : ''}</div>
                    </>
                  )}
                </div>
                {editing?.id !== g.id && (
                  <>
                    <button className="btn sm" aria-expanded={open} onClick={() => setOpenId(open ? null : g.id)}>{manage ? t('Manage members') : t('Members')}</button>
                    {manage && <button className="btn ghost sm icon" title={t('Rename')} aria-label={t('Rename {name}', { name: g.name })} onClick={() => setEditing({ id: g.id, name: g.name })}><Pencil size={14} /></button>}
                    {manage && (
                      <button className="btn ghost sm icon danger" title={t('Delete group')} aria-label={t('Delete {name}', { name: g.name })} onClick={() => {
                        if (!confirm(t('Delete the group {name}? Designs shared with it are no longer shared with its members (unless shared another way).', { name: g.name }))) return;
                        void act(groupsApi.remove(ws.id, g.id), t('Group deleted'));
                      }}><Trash2 size={14} /></button>
                    )}
                  </>
                )}
              </div>
              {open && (
                <div className="col" style={{ gap: 4, paddingLeft: 38 }}>
                  {(manage ? members : people).map((m) => {
                    const inGroup = g.memberIds.includes(m.id);
                    return (
                      <label key={m.id} className="row" style={{ fontSize: 13, gap: 8, cursor: manage ? 'pointer' : 'default' }}>
                        {manage && (
                          <input type="checkbox" checked={inGroup} onChange={(e) => void act(e.target.checked ? groupsApi.addMembers(ws.id, g.id, [m.id]) : groupsApi.removeMember(ws.id, g.id, m.id))} />
                        )}
                        <Avatar name={m.name} color={m.color} size={22} />
                        <span className="grow">{m.name} <span className="faint">{m.email}</span></span>
                      </label>
                    );
                  })}
                  {!manage && !people.length && <span className="faint" style={{ fontSize: 13 }}>{t('No members yet.')}</span>}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
