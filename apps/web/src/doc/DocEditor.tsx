/**
 * Docs (DC-001..010): a flowing rich-text document, collaboratively edited in the same Yjs document as
 * the design (fragment "docBody") — real-time co-editing, presence carets and offline editing come free.
 * Headings, lists, task lists, quotes, code, tables, images (uploads), links, colours, highlights,
 * alignment, a "/" command menu, an outline and word count. Download as a vector PDF or convert to a design.
 * Rich blocks (DC-030): embedded designs, charts, callouts, a table of contents, page breaks and video
 * embeds; Word/Markdown download and Markdown/Word import (DC-031..033).
 */
import type { HocuspocusProvider } from '@hocuspocus/provider';
import Collaboration from '@tiptap/extension-collaboration';
import CollaborationCaret from '@tiptap/extension-collaboration-caret';
import Highlight from '@tiptap/extension-highlight';
import Image from '@tiptap/extension-image';
import { TaskItem, TaskList } from '@tiptap/extension-list';
import { TableKit } from '@tiptap/extension-table';
import TextAlign from '@tiptap/extension-text-align';
import { Color, FontFamily, TextStyle } from '@tiptap/extension-text-style';
import { CharacterCount, Placeholder } from '@tiptap/extensions';
import { EditorContent, useEditor, useEditorState, type Editor as TipTap } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import {
  AlignCenter, AlignLeft, AlignRight, BarChart3, Bold, ChevronLeft, Code, Download, FileUp, Heading1, Heading2, Heading3, Highlighter, Image as ImageIcon, Info, Italic,
  LayoutTemplate, Link2, List, ListChecks, ListOrdered, ListTree, Minus, Quote, Redo2, SeparatorHorizontal, Share2, Strikethrough, Table as TableIcon, Underline, Undo2, Video, Wand2,
} from 'lucide-react';
import { CALLOUT_TONES, type CalloutTone } from '@oc/engine';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import type * as Y from 'yjs';
import { api, assetUrl, download, type DesignSummary, type Role, type User } from '../api';
import { Modal, PopoverButton, useToast } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { CALLOUT_LABEL, DOC_BLOCKS, DesignPicker } from './blocks';
import { fileToHtml, looksLikeMarkdown, markdownToHtml, takePendingImport } from './importDoc';

interface Props { ydoc: Y.Doc; provider: HocuspocusProvider; design: DesignSummary & { role: Role }; user: User }

/** Commands that need a dialog first (pick a design, paste a video link). */
interface CmdUi { pickDesign(): void; askVideo(): void }
type Cmd = { id: string; label: string; hint: string; icon: React.ReactNode; run: (e: TipTap, ui: CmdUi) => void };
const COMMANDS: Cmd[] = [
  { id: 'h1', label: tk('Heading 1'), hint: tk('Big section title'), icon: <Heading1 size={16} />, run: (e) => e.chain().focus().toggleHeading({ level: 1 }).run() },
  { id: 'h2', label: tk('Heading 2'), hint: tk('Medium title'), icon: <Heading2 size={16} />, run: (e) => e.chain().focus().toggleHeading({ level: 2 }).run() },
  { id: 'h3', label: tk('Heading 3'), hint: tk('Small title'), icon: <Heading3 size={16} />, run: (e) => e.chain().focus().toggleHeading({ level: 3 }).run() },
  { id: 'ul', label: tk('Bulleted list'), hint: '•', icon: <List size={16} />, run: (e) => e.chain().focus().toggleBulletList().run() },
  { id: 'ol', label: tk('Numbered list'), hint: '1.', icon: <ListOrdered size={16} />, run: (e) => e.chain().focus().toggleOrderedList().run() },
  { id: 'task', label: tk('Checklist'), hint: '☐', icon: <ListChecks size={16} />, run: (e) => e.chain().focus().toggleTaskList().run() },
  { id: 'quote', label: tk('Quote'), hint: '“', icon: <Quote size={16} />, run: (e) => e.chain().focus().toggleBlockquote().run() },
  { id: 'code', label: tk('Code block'), hint: '{ }', icon: <Code size={16} />, run: (e) => e.chain().focus().toggleCodeBlock().run() },
  { id: 'table', label: tk('Table'), hint: '3 × 3', icon: <TableIcon size={16} />, run: (e) => e.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run() },
  { id: 'hr', label: tk('Divider'), hint: '—', icon: <Minus size={16} />, run: (e) => e.chain().focus().setHorizontalRule().run() },
  { id: 'callout', label: tk('Callout'), hint: 'ℹ', icon: <Info size={16} />, run: (e) => e.chain().focus().toggleCallout('info').run() },
  { id: 'design', label: tk('Design'), hint: tk('Embed one of your designs'), icon: <LayoutTemplate size={16} />, run: (_e, ui) => ui.pickDesign() },
  { id: 'chart', label: tk('Chart'), hint: tk('Bar, line, pie…'), icon: <BarChart3 size={16} />, run: (e) => e.chain().focus().insertDocChart('column').run() },
  { id: 'toc', label: tk('Table of contents'), hint: tk('Lists your headings'), icon: <ListTree size={16} />, run: (e) => e.chain().focus().insertTableOfContents().run() },
  { id: 'pagebreak', label: tk('Page break'), hint: '⤓', icon: <SeparatorHorizontal size={16} />, run: (e) => e.chain().focus().insertPageBreak().run() },
  { id: 'video', label: tk('Video'), hint: 'YouTube, Vimeo, Loom', icon: <Video size={16} />, run: (_e, ui) => ui.askVideo() },
];

export function DocEditor({ ydoc, provider, design, user }: Props) {
  const nav = useNavigate();
  const toast = useToast();
  const readOnly = design.role === 'view' || design.role === 'comment';
  const [title, setTitle] = useState(design.title);
  const [slash, setSlash] = useState<{ top: number; left: number; q: string; i: number } | null>(null);
  const [exporting, setExporting] = useState(false);
  const [dialog, setDialog] = useState<'design' | 'video' | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const importRef = useRef<HTMLInputElement>(null);
  const ui: CmdUi = { pickDesign: () => setDialog('design'), askVideo: () => setDialog('video') };

  const upload = async (files: File[], ed: TipTap) => {
    for (const f of files.filter((x) => x.type.startsWith('image/'))) {
      try {
        const { asset } = await api.upload(f);
        ed.chain().focus().setImage({ src: assetUrl(asset.id), alt: f.name }).run();
      } catch (e) {
        toast(t('Upload failed: {message}', { message: (e as Error).message }));
      }
    }
  };

  const editor = useEditor({
    editable: !readOnly,
    extensions: [
      StarterKit.configure({ undoRedo: false, link: { openOnClick: readOnly, autolink: true } }),
      Collaboration.configure({ document: ydoc, field: 'docBody' }),
      CollaborationCaret.configure({ provider, user: { name: user.name, color: user.color } }),
      Placeholder.configure({ placeholder: ({ node }) => (node.type.name === 'heading' ? t('Heading') : t('Type “/” for commands, or just start writing…')) }),
      TextStyle, Color, FontFamily, Highlight.configure({ multicolor: true }),
      TextAlign.configure({ types: ['heading', 'paragraph'] }),
      TaskList, TaskItem.configure({ nested: true }),
      TableKit.configure({ table: { resizable: true } }),
      Image.configure({ allowBase64: false }),
      CharacterCount,
      ...DOC_BLOCKS,
    ],
    editorProps: {
      attributes: { class: 'doc-prose', 'aria-label': t('Document') },
      handleDrop: (_view, ev) => {
        const files = [...((ev as DragEvent).dataTransfer?.files ?? [])];
        if (files.length && editor) {
          ev.preventDefault();
          void upload(files, editor);
          return true;
        }
        return false;
      },
      handlePaste: (_view, ev): boolean => {
        const files = [...(ev.clipboardData?.files ?? [])];
        if (files.length && editor) {
          void upload(files, editor);
          return true;
        }
        const text = ev.clipboardData?.getData('text/plain') ?? '';
        // Rich HTML (from a browser or Word) pastes as is; plain text, or code-editor HTML without structure, may be Markdown.
        const html = ev.clipboardData?.getData('text/html') ?? '';
        if (!editor || /<(h[1-6]|ul|ol|li|strong|b|em|i|table|blockquote|pre|a)\b/i.test(html)) return false;
        // A lone video link becomes a player; Markdown becomes formatted blocks.
        if (/^https?:\/\/\S+$/.test(text.trim()) && editor.can().insertVideoEmbed(text.trim())) {
          return editor.chain().focus().insertVideoEmbed(text.trim()).run();
        }
        if (looksLikeMarkdown(text)) {
          editor.chain().focus().insertContent(markdownToHtml(text)).run();
          return true;
        }
        return false;
      },
    },
  });

  // A doc just created by "Import file" (Markdown/Word): place its content once.
  useEffect(() => {
    if (!editor || readOnly) return;
    const html = takePendingImport(design.id);
    if (html && editor.isEmpty) editor.commands.setContent(html);
  }, [editor, readOnly, design.id]);

  // Slash menu: filter by what's typed after "/", Enter applies.
  const st = useEditorState({
    editor,
    selector: ({ editor: e }) => (e ? {
      text: e.state.selection.$from.parent.textContent,
      words: e.storage.characterCount?.words?.() ?? 0,
      chars: e.storage.characterCount?.characters?.() ?? 0,
      headings: (() => {
        const out: Array<{ level: number; text: string; pos: number }> = [];
        e.state.doc.descendants((n, pos) => {
          if (n.type.name === 'heading') out.push({ level: n.attrs.level as number, text: n.textContent, pos });
        });
        return out;
      })(),
    } : null),
  });
  // A paragraph that starts with "/" (typed, pasted or dictated) opens the command menu.
  useEffect(() => {
    if (!st || !editor || readOnly) return;
    const t = st.text;
    const isCmd = t.startsWith('/') && t.length <= 24 && !t.includes(' ');
    if (!isCmd) {
      if (slash) setSlash(null);
      return;
    }
    if (!slash) {
      const coords = editor.view.coordsAtPos(editor.state.selection.from);
      setSlash({ top: coords.bottom + 6, left: coords.left, q: t.slice(1), i: 0 });
    } else if (t.slice(1) !== slash.q) setSlash({ ...slash, q: t.slice(1), i: 0 });
  }, [st?.text, slash, st, editor, readOnly]);
  const matches = useMemo(() => COMMANDS.filter((c) => !slash?.q || t(c.label).toLowerCase().includes(slash.q.toLowerCase()) || c.label.toLowerCase().includes(slash.q.toLowerCase())), [slash?.q]);
  const runSlash = (c: Cmd) => {
    if (!editor) return;
    const { $from } = editor.state.selection;
    editor.chain().focus().deleteRange({ from: $from.start(), to: $from.pos }).run();
    c.run(editor, ui);
    setSlash(null);
  };
  useEffect(() => {
    if (!slash) return;
    const k = (e: KeyboardEvent) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); setSlash((s) => s && { ...s, i: Math.min(matches.length - 1, s.i + 1) }); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); setSlash((s) => s && { ...s, i: Math.max(0, s.i - 1) }); }
      else if (e.key === 'Enter' && matches[slash.i]) { e.preventDefault(); e.stopPropagation(); runSlash(matches[slash.i]!); }
      else if (e.key === 'Escape') setSlash(null);
    };
    window.addEventListener('keydown', k, true);
    return () => window.removeEventListener('keydown', k, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [slash, matches]);

  const saveTitle = async () => {
    const t = title.trim();
    if (t && t !== design.title) await api.updateDesign(design.id, { title: t }).catch(() => {});
  };

  const exportAs = async (format: 'pdf' | 'docx' | 'md') => {
    setExporting(true);
    try {
      download(await api.exportDesign(design.id, { format }), `${title.replace(/[^\w\- ]+/g, '').trim() || 'document'}.${format}`);
    } catch (e) {
      toast(t('Export failed: {message}', { message: (e as Error).message }));
    } finally {
      setExporting(false);
    }
  };
  const convert = async () => {
    const r = await fetch(`/api/designs/${design.id}/doc/convert`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
    if (!r.ok) return toast(t('Conversion failed.'));
    const { id } = (await r.json()) as { id: string };
    toast(t('Created a design from this doc'));
    nav(`/design/${id}`);
  };

  if (!editor) return null;
  const btn = (on: boolean, title2: string, icon: React.ReactNode, fn: () => void) => (
    <button className={`btn ghost sm icon ${on ? 'active' : ''}`} title={title2} aria-label={title2} aria-pressed={on} onMouseDown={(e) => e.preventDefault()} onClick={fn}>{icon}</button>
  );
  const block = editor.isActive('heading', { level: 1 }) ? 'h1' : editor.isActive('heading', { level: 2 }) ? 'h2' : editor.isActive('heading', { level: 3 }) ? 'h3' : 'p';

  return (
    <div className="doc-shell">
      <header className="editor-header">
        <button className="btn ghost sm" onClick={() => nav('/')} aria-label={t('Home')}><ChevronLeft size={16} /><span className="hide-mobile"> {t('Home')}</span></button>
        <span className="surface-label hide-mobile">{t('Doc')}</span>
        <div className="grow" />
        <input className="title-input" value={title} disabled={readOnly} onChange={(e) => setTitle(e.target.value)} onBlur={saveTitle} onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()} aria-label={t('Document title')} />
        <div className="grow" />
        <button className="btn ghost sm" onClick={convert} disabled={readOnly} title={t('Lay this doc out as editable design pages')}><Wand2 size={16} /><span className="hide-mobile"> {t('Convert to design')}</span></button>
        <PopoverButton title={t('Download')} className="btn ghost sm" label={<><Download size={16} /><span className="hide-mobile"> {exporting ? t('Preparing…') : t('Download')}</span></>} content={(close) => (
          <div className="col" style={{ width: 240, gap: 2 }} role="menu">
            {([['pdf', t('PDF — print-ready pages')], ['docx', t('Word (.docx) — editable document')], ['md', t('Markdown (.md) — plain text')]] as const).map(([f, label]) => (
              <button key={f} role="menuitem" className="btn ghost sm" style={{ justifyContent: 'flex-start' }} disabled={exporting} onClick={() => { close(); void exportAs(f); }}>{label}</button>
            ))}
          </div>
        )} />
        <button className="btn sm white" onClick={() => void navigator.clipboard.writeText(location.href).then(() => toast(t('Link copied — share access from Home')))}><Share2 size={16} /><span className="hide-narrow"> {t('Share')}</span></button>
      </header>
      {!readOnly && (
        <div className="ctx-toolbar doc-toolbar" role="toolbar" aria-label={t('Formatting')}>
          {btn(false, t('Undo'), <Undo2 size={16} />, () => editor.chain().focus().undo().run())}
          {btn(false, t('Redo'), <Redo2 size={16} />, () => editor.chain().focus().redo().run())}
          <div className="sep" />
          <select className="input" style={{ width: 130 }} value={block} aria-label={t('Text style')} onChange={(e) => {
            const v = e.target.value;
            const c = editor.chain().focus();
            (v === 'p' ? c.setParagraph() : c.setHeading({ level: Number(v[1]) as 1 | 2 | 3 })).run();
          }}>
            <option value="p">{t('Normal text')}</option><option value="h1">{t('Heading 1')}</option><option value="h2">{t('Heading 2')}</option><option value="h3">{t('Heading 3')}</option>
          </select>
          <select className="input" style={{ width: 130 }} aria-label={t('Font')} value={(editor.getAttributes('textStyle').fontFamily as string | undefined) ?? ''} onChange={(e) => (e.target.value ? editor.chain().focus().setFontFamily(e.target.value).run() : editor.chain().focus().unsetFontFamily().run())}>
            <option value="">{t('Default font')}</option>
            {['Inter', 'Lora', 'Merriweather', 'Playfair Display', 'Montserrat', 'Roboto', 'Open Sans', 'Source Serif 4', 'JetBrains Mono'].map((f) => <option key={f} value={f}>{f}</option>)}
          </select>
          <div className="sep" />
          {btn(editor.isActive('bold'), t('Bold (⌘B)'), <Bold size={16} />, () => editor.chain().focus().toggleBold().run())}
          {btn(editor.isActive('italic'), t('Italic (⌘I)'), <Italic size={16} />, () => editor.chain().focus().toggleItalic().run())}
          {btn(editor.isActive('underline'), t('Underline (⌘U)'), <Underline size={16} />, () => editor.chain().focus().toggleUnderline().run())}
          {btn(editor.isActive('strike'), t('Strikethrough'), <Strikethrough size={16} />, () => editor.chain().focus().toggleStrike().run())}
          <label className="btn ghost sm icon" title={t('Text colour')}><input type="color" aria-label={t('Text colour')} style={{ width: 18, height: 18, border: 0, padding: 0, background: 'none' }} onChange={(e) => editor.chain().focus().setColor(e.target.value).run()} /></label>
          {btn(editor.isActive('highlight'), t('Highlight'), <Highlighter size={16} />, () => editor.chain().focus().toggleHighlight({ color: '#fde68a' }).run())}
          <PopoverButton title={t('Link')} label={<Link2 size={16} />} className={`btn ghost sm icon ${editor.isActive('link') ? 'active' : ''}`} content={(close) => (
            <LinkForm initial={(editor.getAttributes('link').href as string | undefined) ?? ''} onSubmit={(url) => {
              const c = editor.chain().focus().extendMarkRange('link');
              (url ? c.setLink({ href: /^(https?:|mailto:)/.test(url) ? url : `https://${url}` }) : c.unsetLink()).run();
              close();
            }} />
          )} />
          <div className="sep" />
          {btn(editor.isActive({ textAlign: 'left' }), t('Align left'), <AlignLeft size={16} />, () => editor.chain().focus().setTextAlign('left').run())}
          {btn(editor.isActive({ textAlign: 'center' }), t('Align centre'), <AlignCenter size={16} />, () => editor.chain().focus().setTextAlign('center').run())}
          {btn(editor.isActive({ textAlign: 'right' }), t('Align right'), <AlignRight size={16} />, () => editor.chain().focus().setTextAlign('right').run())}
          <div className="sep" />
          {btn(editor.isActive('bulletList'), t('Bulleted list'), <List size={16} />, () => editor.chain().focus().toggleBulletList().run())}
          {btn(editor.isActive('orderedList'), t('Numbered list'), <ListOrdered size={16} />, () => editor.chain().focus().toggleOrderedList().run())}
          {btn(editor.isActive('taskList'), t('Checklist'), <ListChecks size={16} />, () => editor.chain().focus().toggleTaskList().run())}
          {btn(editor.isActive('blockquote'), t('Quote'), <Quote size={16} />, () => editor.chain().focus().toggleBlockquote().run())}
          {btn(editor.isActive('callout'), t('Callout'), <Info size={16} />, () => editor.chain().focus().toggleCallout('info').run())}
          {editor.isActive('callout') && (
            <select className="input" style={{ width: 100 }} aria-label={t('Callout style')} value={(editor.getAttributes('callout').tone as string | undefined) ?? 'info'} onChange={(e) => editor.chain().focus().setCalloutTone(e.target.value as CalloutTone).run()}>
              {(Object.keys(CALLOUT_TONES) as CalloutTone[]).map((k) => <option key={k} value={k}>{t(CALLOUT_LABEL[k])}</option>)}
            </select>
          )}
          {btn(false, t('Insert image'), <ImageIcon size={16} />, () => fileRef.current?.click())}
          {btn(false, t('Insert table'), <TableIcon size={16} />, () => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run())}
          {btn(false, t('Insert chart'), <BarChart3 size={16} />, () => editor.chain().focus().insertDocChart('column').run())}
          {btn(false, t('Embed a design'), <LayoutTemplate size={16} />, () => setDialog('design'))}
          {btn(false, t('Import Markdown or Word file'), <FileUp size={16} />, () => importRef.current?.click())}
          {editor.isActive('table') && (
            <>
              <button className="btn ghost sm" onClick={() => editor.chain().focus().addRowAfter().run()}>{t('+ Row')}</button>
              <button className="btn ghost sm" onClick={() => editor.chain().focus().addColumnAfter().run()}>{t('+ Column')}</button>
              <button className="btn ghost sm" onClick={() => editor.chain().focus().deleteTable().run()}>{t('Delete table')}</button>
            </>
          )}
          <input ref={fileRef} type="file" accept="image/*" hidden multiple onChange={(e) => { void upload([...(e.target.files ?? [])], editor); e.target.value = ''; }} />
          <input ref={importRef} type="file" accept=".md,.markdown,.docx,text/markdown,application/vnd.openxmlformats-officedocument.wordprocessingml.document" hidden onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = '';
            if (f) void fileToHtml(f).then(({ html }) => editor.chain().focus().insertContent(html).run()).catch((err: Error) => toast(t('Import failed: {message}', { message: err.message })));
          }} />
        </div>
      )}
      <div className="doc-body">
        <aside className="doc-outline" aria-label={t('Outline')}>
          <div className="faint" style={{ fontSize: 11, textTransform: 'uppercase', letterSpacing: '.08em', fontWeight: 700, padding: '0 8px 6px' }}>{t('Outline')}</div>
          {st?.headings.length ? st.headings.map((h, i) => (
            <button key={i} style={{ paddingLeft: 8 + (h.level - 1) * 12 }} onClick={() => {
              editor.chain().focus().setTextSelection(h.pos + 1).run();
              editor.view.domAtPos(h.pos + 1).node.parentElement?.scrollIntoView({ behavior: 'smooth', block: 'center' });
            }}>{h.text || t('Untitled')}</button>
          )) : <p className="faint" style={{ fontSize: 12, padding: '0 8px' }}>{t('Headings appear here.')}</p>}
        </aside>
        <main className="doc-scroll">
          <div className="doc-page"><EditorContent editor={editor} /></div>
          <div className="faint doc-count">{tn(st?.words ?? 0, '{n} word', '{n} words')} · {tn(st?.chars ?? 0, '{n} character', '{n} characters')}</div>
        </main>
      </div>
      {dialog === 'design' && (
        <DesignPicker exclude={design.id} onClose={() => setDialog(null)} onPick={(d) => {
          editor.chain().focus().insertDesignEmbed({ designId: d.id, title: d.title, pageId: '1' }).run();
          setDialog(null);
        }} />
      )}
      {dialog === 'video' && (
        <Modal title={t('Embed a video')} onClose={() => setDialog(null)} size="sm">
          <VideoForm onSubmit={(url) => {
            if (!editor.chain().focus().insertVideoEmbed(url).run()) return toast(t('Paste a YouTube, Vimeo or Loom link.'));
            setDialog(null);
          }} />
        </Modal>
      )}
      {slash && matches.length > 0 && (
        <div className="popover slash-menu" style={{ top: slash.top, left: slash.left }} role="listbox" aria-label={t('Insert block')}>
          {matches.map((c, i) => (
            <button key={c.id} role="option" aria-selected={i === slash.i} className={i === slash.i ? 'on' : ''} onMouseDown={(e) => { e.preventDefault(); runSlash(c); }}>
              {c.icon}<span className="grow">{t(c.label)}</span><span className="faint">{t(c.hint)}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function VideoForm({ onSubmit }: { onSubmit: (url: string) => void }) {
  const [url, setUrl] = useState('');
  return (
    <form className="col" onSubmit={(e) => { e.preventDefault(); if (url.trim()) onSubmit(url.trim()); }}>
      <input className="input" placeholder="https://www.youtube.com/watch?v=…" value={url} onChange={(e) => setUrl(e.target.value)} autoFocus aria-label={t('Video link')} />
      <button className="btn primary sm">{t('Embed')}</button>
    </form>
  );
}

function LinkForm({ initial, onSubmit }: { initial: string; onSubmit: (url: string) => void }) {
  const [url, setUrl] = useState(initial);
  return (
    <form className="row" style={{ width: 320 }} onSubmit={(e) => { e.preventDefault(); onSubmit(url.trim()); }}>
      <input className="input grow" placeholder="https://" value={url} onChange={(e) => setUrl(e.target.value)} autoFocus />
      <button className="btn primary sm">{url ? t('Apply') : t('Remove')}</button>
    </form>
  );
}
