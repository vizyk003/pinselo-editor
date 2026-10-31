/**
 * Editor File menu (g88) and Projects panel (g84) client. Thin wrappers over the existing routes, so the editor
 * shows exactly what the dashboard does and every permission check stays on the server:
 * folders (GET /api/folders), folder contents (GET /api/designs?filter=folder), search (GET /api/search),
 * another design's last saved snapshot, read-only (GET /api/designs/:id/doc, needs view access), and the
 * star / move / trash updates (PATCH and DELETE /api/designs/:id).
 */
import type { DesignDoc } from '@oc/engine';
import { api, type DesignSummary, type Folder } from '../api';
import { searchApi, type SearchFolder } from '../searchApi';

export interface ProjectsSearch { designs: DesignSummary[]; folders: SearchFolder[] }

export const editorFilesApi = {
  /** My folders plus the team folders of teams I belong to. */
  folders: (): Promise<Folder[]> => api.folders().then((r) => r.folders),

  /**
   * Designs directly in a folder: everything in it the user can open (a team folder holds the team's designs);
   * null = my designs that are not in any folder. Same listing as the Projects page.
   */
  designsIn: (folderId: string | null): Promise<DesignSummary[]> =>
    api.designs(folderId ? { filter: 'folder', folder: folderId } : { filter: 'folder' }).then((r) => r.designs),

  /** Designs and folders matching `q`, with the same visibility as browsing. */
  search: (q: string, signal?: AbortSignal): Promise<ProjectsSearch> =>
    searchApi.search(q, { type: 'all' }, 30, signal).then((r) => ({ designs: r.designs, folders: r.folders })),

  /** Another design's last saved document, read-only (no live connection, nothing is written back). */
  sourceDoc: (id: string): Promise<DesignDoc> => api.designDoc(id).then((r) => r.doc),

  setStarred: (id: string, starred: boolean) => api.updateDesign(id, { starred }),
  /** Into a folder (needs edit rights) or out of all folders with null (owner only). */
  moveTo: (id: string, folderId: string | null) => api.updateDesign(id, { folderId }),
  /** Owner only; restorable from the trash. */
  trash: (id: string) => api.trashDesign(id),
};
