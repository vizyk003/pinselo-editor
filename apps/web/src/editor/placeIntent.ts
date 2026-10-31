/**
 * `/design/:id?place=<assetId>` (g15): a design started from an upload opens with that photo or video filling the page.
 * The intent is resumable (a reload or a second tab re-reads the URL) and idempotent (skipped once the asset is on the page).
 */
import type { Editor } from '@oc/engine';
import { makeImage, makeVideo, type AssetRef } from '@oc/engine';
import { uploadsApi } from '../api/uploads';
import { coverBox, toAssetRef } from '../pages/home/startMedia';

/** Place `asset` edge to edge on the active page (cover fit, centred). */
export function placeFullBleed(editor: Editor, asset: AssetRef) {
  editor.apply([{ op: 'asset', asset }], true);
  const { w: W, h: H } = editor.doc.meta.dims;
  const b = coverBox(asset.w, asset.h, W, H);
  const xf = { x: 0, y: 0, w: b.w, h: b.h, rot: 0 };
  const name = asset.name ? { name: asset.name } : {};
  if (asset.kind === 'video') {
    const el = makeVideo({ xf, asset: asset.id, durationMs: asset.durationMs ?? 5000, ...name });
    if (asset.animated) el.clip = { ...el.clip, loop: true, muted: true };
    editor.insert([el], [W / 2, H / 2]);
  } else {
    editor.insert([makeImage({ xf, asset: asset.id, ...name })], [W / 2, H / 2]);
  }
}

/** Resolve and place the asset named by `?place=`; false when it is already placed or isn't one of the user's uploads. */
export async function runPlaceIntent(editor: Editor, assetId: string): Promise<boolean> {
  if (!/^[\w-]{1,80}$/.test(assetId)) return false;
  if (Object.values(editor.doc.elements).some((e) => 'asset' in e && e.asset === assetId)) return false;
  // Looked up by id: the Uploads page can start a design from an upload of any age, not just the newest 500.
  const { assets } = await uploadsApi.list({ id: assetId, limit: 1 });
  const row = assets.find((a) => a.id === assetId);
  const asset = row ? toAssetRef(row as unknown as Record<string, unknown>) : null;
  if (!asset || asset.kind === 'audio') return false;
  placeFullBleed(editor, asset);
  return true;
}
