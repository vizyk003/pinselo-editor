/**
 * Uploading many files at once (g19, g87): dropped folders are walked, unsupported files skipped, and the
 * rest go one by one through the regular upload (`api.upload`), so quota and safety checks apply to each.
 */
import type { AssetRef } from '@oc/engine';
import { api, ApiError } from '../api';
import { isUploadable } from '../pages/home/uploadsModel';

/** Most files taken from one folder or drop; a stray drop of a photo library shouldn't queue thousands of uploads. */
export const MAX_FILES = 200;
const MAX_DEPTH = 8;

const readFile = (e: FileSystemFileEntry) => new Promise<File>((res, rej) => e.file(res, rej));
const readBatch = (r: FileSystemDirectoryReader) => new Promise<FileSystemEntry[]>((res, rej) => r.readEntries(res, rej));

async function walk(entry: FileSystemEntry, out: File[], depth: number): Promise<void> {
  if (out.length >= MAX_FILES) return;
  if (entry.isFile) {
    out.push(await readFile(entry as FileSystemFileEntry));
    return;
  }
  if (!entry.isDirectory || depth >= MAX_DEPTH) return;
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  // readEntries returns the listing in batches until it comes back empty.
  for (let batch = await readBatch(reader); batch.length && out.length < MAX_FILES; batch = await readBatch(reader)) {
    for (const child of batch) await walk(child, out, depth + 1);
  }
}

/**
 * Files from a drop, including the contents of dropped folders. Must be called synchronously from the drop
 * handler (the DataTransfer is emptied once the event returns); the walk itself is async.
 */
export function filesFromDrop(dt: DataTransfer): Promise<File[]> {
  const entries = [...dt.items].flatMap((i) => (i.kind === 'file' ? [i.webkitGetAsEntry?.() ?? null] : [])).filter((e): e is FileSystemEntry => Boolean(e));
  const plain = [...dt.files];
  if (!entries.some((e) => e.isDirectory)) return Promise.resolve(plain);
  const out: File[] = [];
  return (async () => {
    for (const e of entries) await walk(e, out, 0);
    return out;
  })();
}

export interface UploadOutcome { assets: AssetRef[]; failed: Array<{ name: string; message: string }>; skipped: number; stopped: boolean }

/**
 * Uploads the supported files in order, reporting progress. A full storage quota stops the rest (each would
 * fail the same way); other errors are collected per file.
 */
export async function uploadMany(files: File[], onProgress?: (done: number, total: number) => void, upload: (f: File) => Promise<{ asset: AssetRef }> = api.upload): Promise<UploadOutcome> {
  const ok = files.filter(isUploadable).slice(0, MAX_FILES);
  const out: UploadOutcome = { assets: [], failed: [], skipped: files.length - ok.length, stopped: false };
  onProgress?.(0, ok.length);
  for (const [i, f] of ok.entries()) {
    try {
      out.assets.push((await upload(f)).asset);
    } catch (e) {
      out.failed.push({ name: f.name, message: e instanceof ApiError ? e.message : (e as Error).message });
      if (e instanceof ApiError && e.status === 402) {
        out.stopped = true;
        break;
      }
    }
    onProgress?.(i + 1, ok.length);
  }
  return out;
}

/** Marks a file input as a folder picker (React has no typed prop for `webkitdirectory`). */
export const folderInputRef = (el: HTMLInputElement | null) => {
  if (el) el.setAttribute('webkitdirectory', '');
};
