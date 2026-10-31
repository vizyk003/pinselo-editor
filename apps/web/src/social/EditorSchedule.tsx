/** The editor's "Schedule" entry point: feeds the open design's pages and text into the dialog. */
import { bodyPlainText, sortedPages, videoSlots, type TextEl } from '@oc/engine';
import { useMemo } from 'react';
import { useDoc, useEd } from '../editor/context';
import { ScheduleDialog } from './ScheduleDialog';

export function EditorScheduleDialog({ onClose }: { onClose: () => void }) {
  const { design } = useEd();
  const doc = useDoc();
  const { texts, video, pageCount } = useMemo(() => {
    const els = Object.values(doc.elements).filter((e) => !e.deleted);
    return {
      texts: els.filter((e): e is TextEl => e.type === 'text').map((e) => bodyPlainText(e.body)),
      video: els.some((e) => videoSlots(e).length > 0) || doc.meta.surface === 'video',
      pageCount: sortedPages(doc).length,
    };
  }, [doc]);
  return <ScheduleDialog designId={design.id} title={doc.meta.title} pageCount={pageCount} texts={texts} video={video} onClose={onClose} />;
}
