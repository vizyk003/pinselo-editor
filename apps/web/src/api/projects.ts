/** Projects hub client (plan 11 g17/g18/g20): nested folders, folder stars, sorted/filtered design lists and bulk actions. */
import { ApiError, type DesignSummary, type Folder } from '../api';

/** A folder as GET /api/folders lists it (a superset of `Folder`, so it can go wherever a `Folder` is expected). */
export interface ProjectFolder extends Folder {
  created_at: string; owner_id: string; owner_name: string; subfolders: number; starred: boolean;
  /** Rename, move and delete it. */
  can_manage: boolean;
  /** Add subfolders to it (and move folders into it). */
  can_add: boolean;
  /** Put designs in it. */
  can_file: boolean;
}

export type DesignSort = 'edited' | 'oldest' | 'az' | 'za';
export type OwnerFilter = 'anyone' | 'me' | 'shared';
export type BulkAction = 'star' | 'unstar' | 'move' | 'trash' | 'restore' | 'delete';

export interface DesignQuery {
  filter: string; folder?: string; workspace?: string; q?: string; surface?: string; sort?: DesignSort; owner?: OwnerFilter;
  /** With `folder`: include its subfolders. Without: all of Projects. */
  deep?: boolean;
}

async function req<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method, credentials: 'same-origin',
    ...(body !== undefined ? { body: JSON.stringify(body), headers: { 'content-type': 'application/json' } } : {}),
  });
  if (!res.ok) {
    let j: { error?: string; message?: string } = {};
    try {
      j = (await res.json()) as typeof j;
    } catch {
      /* not json */
    }
    throw new ApiError(res.status, j.error ?? 'error', j.message);
  }
  return res.json() as Promise<T>;
}

/** Query string for GET /api/designs; defaults are left out. */
export function designParams(q: DesignQuery): URLSearchParams {
  const p = new URLSearchParams({ filter: q.filter });
  if (q.folder) p.set('folder', q.folder);
  if (q.workspace) p.set('workspace', q.workspace);
  if (q.q?.trim()) p.set('q', q.q.trim());
  if (q.surface) p.set('surface', q.surface);
  if (q.sort && q.sort !== 'edited') p.set('sort', q.sort);
  if (q.owner && q.owner !== 'anyone') p.set('owner', q.owner);
  if (q.deep) p.set('deep', '1');
  return p;
}

export const projectsApi = {
  folders: () => req<{ folders: ProjectFolder[] }>('GET', '/api/folders'),
  createFolder: (b: { name: string; parentId?: string | null; workspaceId?: string }) => req<{ id: string }>('POST', '/api/folders', b),
  updateFolder: (id: string, b: { name?: string; parentId?: string | null; starred?: boolean }) => req<{ ok: true }>('PATCH', `/api/folders/${id}`, b),
  deleteFolder: (id: string) => req<{ ok: true }>('DELETE', `/api/folders/${id}`),
  designs: (q: DesignQuery) => req<{ designs: DesignSummary[] }>('GET', `/api/designs?${designParams(q)}`),
  bulk: (ids: string[], action: BulkAction, folderId?: string | null) =>
    req<{ done: string[]; failed: Array<{ id: string; error: string }> }>('POST', '/api/designs/bulk', { ids, action, ...(folderId !== undefined ? { folderId } : {}) }),
};
