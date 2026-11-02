/** Sharing API (plan 11 batch 2): people search, group shares, team access, slugs, template links, team groups. */
import type { DesignDoc } from '@oc/engine';
import { ApiError, type Role } from '../api';

async function req<T>(method: string, url: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
  const init: RequestInit = { method, credentials: 'same-origin', headers: { ...headers } };
  if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)['content-type'] = 'application/json';
  }
  const res = await fetch(url, init);
  if (!res.ok) {
    let code = 'error';
    let message: string | undefined;
    try {
      const j = (await res.json()) as { error?: string; message?: string };
      code = j.error ?? code;
      message = j.message;
    } catch {
      /* not json */
    }
    throw new ApiError(res.status, code, message);
  }
  return res.json() as Promise<T>;
}

export type ShareRole = Exclude<Role, 'owner'>;
export interface PersonHit { id: string; name: string; email: string; color: string; teammate: boolean; shared_at: string | null }
export interface GroupHit { id: string; name: string; team: string; members: number }
export interface GroupShare extends GroupHit { role: ShareRole }
export interface SharingState {
  role: Role;
  team: { id: string; name: string; access: ShareRole | null } | null;
  groups: GroupShare[];
  slug: string | null;
  /** Owner only. */
  templateLink: { token: string | null; uses: number } | null;
}
export interface TeamGroup { id: string; name: string; created_at: string; memberIds: string[] }
export interface TemplateLinkInfo { title: string; owner: string; width: number; height: number; unit: string; surface: string; pages: number; thumb: boolean }

const enc = encodeURIComponent;

export const sharingApi = {
  state: (id: string) => req<SharingState>('GET', `/api/designs/${id}/sharing`),
  people: (q: string, designId?: string) =>
    req<{ people: PersonHit[]; groups: GroupHit[]; suggested: boolean }>('GET', `/api/sharing/people?q=${enc(q)}${designId ? `&design=${enc(designId)}` : ''}`),
  shareGroup: (id: string, groupId: string, role: ShareRole) => req('POST', `/api/designs/${id}/group-shares`, { groupId, role }),
  unshareGroup: (id: string, groupId: string) => req('DELETE', `/api/designs/${id}/group-shares/${groupId}`),
  setTeamAccess: (id: string, access: ShareRole | null) => req('PUT', `/api/designs/${id}/team-access`, { access }),
  setSlug: (id: string, slug: string | null) => req<{ slug: string | null }>('PUT', `/api/designs/${id}/slug`, { slug }),
  makePrivate: (id: string) => req<{ removed: { people: number; groups: number } }>('POST', `/api/designs/${id}/make-private`, {}),
  enableTemplateLink: (id: string, regenerate = false) => req<{ token: string; uses: number }>('POST', `/api/designs/${id}/template-link`, { regenerate }),
  disableTemplateLink: (id: string) => req('DELETE', `/api/designs/${id}/template-link`),
  templateLink: (token: string) => req<TemplateLinkInfo>('GET', `/api/public/template-links/${enc(token)}`),
  templateThumbUrl: (token: string) => `/api/public/template-links/${enc(token)}/thumbnail.png`,
  useTemplateLink: (token: string) => req<{ id: string }>('POST', `/api/template-links/${enc(token)}/use`, {}),
  /** A public view link by design id or custom slug; the design id comes back only once expiry and password pass. */
  publicDesign: (key: string, password?: string) =>
    req<{ id: string; title: string; owner: string; doc: DesignDoc }>('GET', `/api/public/designs/${enc(key)}`, undefined, password ? { 'x-view-password': password } : {}),
};

export const groupsApi = {
  list: (ws: string) => req<{ groups: TeamGroup[] }>('GET', `/api/workspaces/${ws}/groups`),
  create: (ws: string, name: string, memberIds: string[] = []) => req<{ id: string }>('POST', `/api/workspaces/${ws}/groups`, { name, memberIds }),
  rename: (ws: string, id: string, name: string) => req('PATCH', `/api/workspaces/${ws}/groups/${id}`, { name }),
  remove: (ws: string, id: string) => req('DELETE', `/api/workspaces/${ws}/groups/${id}`),
  addMembers: (ws: string, id: string, userIds: string[]) => req('POST', `/api/workspaces/${ws}/groups/${id}/members`, { userIds }),
  removeMember: (ws: string, id: string, userId: string) => req('DELETE', `/api/workspaces/${ws}/groups/${id}/members/${userId}`),
};
