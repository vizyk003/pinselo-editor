/**
 * Uploads library client (g19, g22, g87): the filtered listing, trash, rename, download and bulk actions
 * of /api/assets. Uploading itself stays `api.upload` (the shared ingest path).
 */
import { ApiError, type UploadedAsset } from '../api';
import { BATCH_MAX, chunk, uploadParams, type UploadCounts, type UploadFilters } from '../pages/home/uploadsModel';

export type { UploadCounts, UploadFilters, UploadSort, UploadType } from '../pages/home/uploadsModel';

export interface UploadRow extends UploadedAsset {
  source?: string;
  attribution?: { provider?: string; author?: string; authorUrl?: string; license?: string; licenseUrl?: string; sourceUrl?: string } | null;
  created_at: string;
}
/** `held`: reported and under review, so it can't be restored or deleted until a moderator decides. */
export interface TrashedUpload extends UploadRow { trashed_at: string; purge_at: string; held?: boolean }
export interface UploadPage { assets: UploadRow[]; usedBytes: number; quotaBytes: number; total: number; counts: UploadCounts }
export interface TrashPage { assets: TrashedUpload[]; total: number; counts: UploadCounts }
export type BatchAction = 'trash' | 'restore' | 'delete';

async function call<T>(method: string, url: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, {
    method, credentials: 'same-origin', signal,
    ...(body !== undefined ? { body: JSON.stringify(body), headers: { 'content-type': 'application/json' } } : {}),
  });
  if (!res.ok) {
    let j: { error?: string; message?: string } = {};
    try {
      j = (await res.json()) as typeof j;
    } catch {
      /* not json */
    }
    // Fair-use limits surface as the global notice, like every other API call (see LimitHost).
    if (res.status === 402) window.dispatchEvent(new CustomEvent('oc:limit', { detail: { message: j.message } }));
    throw new ApiError(res.status, j.error ?? 'error', j.message);
  }
  return res.json() as Promise<T>;
}

export const uploadsApi = {
  list: (f: Partial<UploadFilters> & { limit?: number; offset?: number; id?: string } = {}, signal?: AbortSignal) =>
    call<UploadPage>('GET', `/api/assets?${uploadParams(f)}`, undefined, signal),
  trash: (f: Partial<UploadFilters> & { limit?: number; offset?: number } = {}, signal?: AbortSignal) =>
    call<TrashPage>('GET', `/api/assets/trash?${uploadParams(f)}`, undefined, signal),
  rename: (id: string, name: string) => call<{ ok: true; name: string }>('PATCH', `/api/assets/${id}`, { name }),
  moveToTrash: (id: string) => call<{ ok: true }>('DELETE', `/api/assets/${id}`),
  restore: (id: string) => call<{ ok: true }>('POST', `/api/assets/${id}/restore`),
  deleteForever: (id: string) => call<{ ok: true }>('DELETE', `/api/assets/${id}?permanent=1`),
  /** Ids the action applied to (others weren't the caller's or weren't in the right state). Sent in chunks the API accepts. */
  batch: async (action: BatchAction, ids: string[]) => {
    const done: string[] = [];
    for (const part of chunk(ids, BATCH_MAX)) done.push(...(await call<{ ok: true; ids: string[] }>('POST', '/api/assets/batch', { action, ids: part })).ids);
    return done;
  },
  /** The original file as an attachment. Same origin: the route needs the session cookie. */
  downloadUrl: (id: string) => `/api/assets/${id}/download`,
};

/** Starts a download of the original file without leaving the page. */
export function downloadUpload(id: string) {
  const a = document.createElement('a');
  a.href = uploadsApi.downloadUrl(id);
  a.download = '';
  document.body.appendChild(a);
  a.click();
  a.remove();
}
