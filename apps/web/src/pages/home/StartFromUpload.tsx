/**
 * Start a design from a file (g15, IM-050/060): photos and videos become a design of their pixel size with the media
 * placed full-bleed (via the editor's `?place=` intent); PDF/PPTX import as editable designs and Markdown/Word as docs.
 */
import { SIZE_PRESETS } from '@oc/engine';
import { UploadCloud } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { api, importApi } from '../../api';
import { Modal, useToast } from '../../components/ui';
import { importIntoNewDoc, isDocImport } from '../../doc/importDoc';
import { t } from '../../i18n';
import { track } from '../../lib/analytics';
import { mediaDesignSize, startKind, startsVideo, titleFromFile } from './startMedia';

/** Accept lists for the file pickers. */
export const MEDIA_ACCEPT = 'image/*,video/*';
export const IMPORT_ACCEPT = '.pdf,.pptx,.docx,.md,.markdown,application/pdf,application/vnd.openxmlformats-officedocument.presentationml.presentation,application/vnd.openxmlformats-officedocument.wordprocessingml.document,text/markdown';

/**
 * Turns a dropped or picked file into a new design and opens it. `status` is a progress label while busy.
 * `folderId` files the new design in that folder (Projects '+ Add new' ▸ Upload, g17).
 */
export function useStartFromFile() {
  const nav = useNavigate();
  const toast = useToast();
  const [status, setStatus] = useState<string | null>(null);
  const start = useCallback(async (file: File, only?: 'media', folderId?: string | null) => {
    const kind = startKind(file);
    if (!kind || (only === 'media' && kind !== 'media')) {
      toast(only === 'media' ? t('Choose an image or a video.') : t('This file type isn’t supported. Upload an image, a video, or a PDF, PowerPoint, Word or Markdown file.'));
      return;
    }
    try {
      setStatus(t('Uploading…'));
      if (kind === 'media') {
        const { asset } = await api.upload(file);
        if (asset.kind === 'audio') throw new Error(t('Choose an image or a video.'));
        const video = startsVideo(asset);
        const size = mediaDesignSize(asset.w, asset.h, asset.kind);
        const { id } = await api.createDesign({ ...(video ? { preset: 'video' } : {}), ...size, unit: 'px', title: titleFromFile(file.name), ...(folderId ? { folderId } : {}) });
        track('design_from_upload', { kind: asset.kind });
        nav(`/design/${id}?place=${encodeURIComponent(asset.id)}`);
        return;
      }
      // Markdown and Word files become a new doc (DC-033).
      if (isDocImport(file)) {
        const id = await importIntoNewDoc(file, folderId);
        track('import_done', { kind: file.name.split('.').pop() });
        nav(`/design/${id}`);
        return;
      }
      const { jobId } = await importApi.start(file, folderId);
      for (;;) {
        await new Promise((r) => setTimeout(r, 900));
        const s = await importApi.status(jobId);
        setStatus(`${s.label} · ${Math.round(s.progress * 100)}%`);
        if (s.status === 'error') throw new Error(s.error ?? t('Import failed'));
        if (s.status === 'done' && s.designId) {
          track('import_done', { kind: file.name.split('.').pop(), pages: s.pages });
          nav(`/design/${s.designId}`);
          return;
        }
      }
    } catch (e) {
      toast(e instanceof Error ? e.message : t('Upload failed'));
    } finally {
      setStatus(null);
    }
  }, [nav, toast]);
  return { start, status };
}

/** A hidden file input plus a function that opens it. */
export function useFilePicker(accept: string, onFile: (f: File) => void) {
  const ref = useRef<HTMLInputElement>(null);
  const input = (
    <input ref={ref} type="file" hidden accept={accept}
      onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ''; if (f) onFile(f); }} />
  );
  return { open: () => ref.current?.click(), input };
}

const hasFiles = (e: DragEvent | React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');

/**
 * Drop a file anywhere on the wrapped area to start a design from it; shows the drop hint while dragging.
 */
export function FileDropArea({ onFile, children, disabled }: { onFile: (f: File) => void; children: ReactNode; disabled?: boolean }) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  useEffect(() => {
    // Leaving the window or dropping elsewhere must clear the hint.
    const reset = () => { depth.current = 0; setOver(false); };
    window.addEventListener('dragend', reset);
    window.addEventListener('drop', reset);
    return () => { window.removeEventListener('dragend', reset); window.removeEventListener('drop', reset); };
  }, []);
  return (
    <div
      className="home-drop-area"
      onDragEnter={(e) => { if (disabled || !hasFiles(e)) return; e.preventDefault(); depth.current++; setOver(true); }}
      onDragOver={(e) => { if (disabled || !hasFiles(e)) return; e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }}
      onDragLeave={() => { if (--depth.current <= 0) { depth.current = 0; setOver(false); } }}
      onDrop={(e) => {
        if (disabled || !hasFiles(e)) return;
        e.preventDefault();
        depth.current = 0;
        setOver(false);
        const f = e.dataTransfer.files[0];
        if (f) onFile(f);
      }}
    >
      {children}
      {over && <div className="drop-hint">{t('Drop to start a design from this file')}</div>}
    </div>
  );
}

/**
 * Upload dialog (the Upload / Photo editor tiles, and the `/?create=upload|photo` intent resumed after signing in from
 * the public home, where a file picker can't open without a click).
 */
export function StartUploadDialog({ mode, onClose }: { mode: 'upload' | 'photo'; onClose: () => void }) {
  const { start, status } = useStartFromFile();
  const photo = mode === 'photo';
  const run = (f: File) => void start(f, photo ? 'media' : undefined);
  const picker = useFilePicker(photo ? MEDIA_ACCEPT : `${MEDIA_ACCEPT},${IMPORT_ACCEPT}`, run);
  return (
    <Modal title={photo ? t('Edit a photo') : t('Upload a file')} onClose={onClose} size="md">
      <FileDropArea onFile={run} disabled={Boolean(status)}>
        <button className="home-dropzone" onClick={picker.open} disabled={Boolean(status)}>
          <UploadCloud size={32} />
          <strong>{status ?? (photo ? t('Choose a photo or video') : t('Choose a file'))}</strong>
          <span className="muted">{photo
            ? t('or drop it here. The design takes the size of your photo or video.')
            : t('or drop it here: an image or video starts a design of its size; PDF, PowerPoint, Word and Markdown files are imported.')}</span>
        </button>
      </FileDropArea>
      {picker.input}
    </Modal>
  );
}
