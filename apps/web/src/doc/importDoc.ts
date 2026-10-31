/**
 * Markdown / Word into docs (DC-033). Markdown is converted in the browser (marked → HTML); Word files
 * go through the API (mammoth → HTML, pictures become uploads). The HTML is then normalised to the doc's
 * block markup (task lists, GFM callouts, page breaks) and parsed by Tiptap's own schema — anything the
 * schema doesn't know (scripts, styles, event handlers) is dropped.
 */
import { marked } from 'marked';
import { api } from '../api';

const KEY = (id: string) => `oc:doc-import:${id}`;

/** Markdown → HTML in the doc's block markup. */
export function markdownToHtml(md: string): string {
  return normalizeImportedHtml(marked.parse(md, { async: false, gfm: true, breaks: false }) as string);
}

/** Does pasted plain text look like Markdown (worth converting)? */
export function looksLikeMarkdown(text: string): boolean {
  if (!text.includes('\n') && !/^#{1,6} /.test(text)) return false;
  return /^(#{1,6} |\s*[-*+] \[[ xX]\] |\s*[-*+] |\s*\d+\. |> |```|~~~|\|.+\|\s*$)/m.test(text) || /\*\*[^*]+\*\*|\[[^\]]+\]\([^)]+\)/.test(text);
}

const CALLOUT: Record<string, string> = { NOTE: 'info', TIP: 'success', IMPORTANT: 'note', WARNING: 'warning', CAUTION: 'danger' };

export function normalizeImportedHtml(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  // GFM task lists: <li><input type=checkbox checked> text</li> → Tiptap task items.
  for (const ul of [...doc.querySelectorAll('ul')]) {
    const items = [...ul.children].filter((c) => c.tagName === 'LI');
    if (!items.length || !items.every((li) => li.querySelector(':scope > input[type=checkbox], :scope > p > input[type=checkbox]'))) continue;
    ul.setAttribute('data-type', 'taskList');
    for (const li of items) {
      const box = li.querySelector('input[type=checkbox]') as HTMLInputElement;
      li.setAttribute('data-type', 'taskItem');
      li.setAttribute('data-checked', box.hasAttribute('checked') ? 'true' : 'false');
      box.remove();
      if (!li.querySelector(':scope > p')) {
        const p = doc.createElement('p');
        while (li.firstChild && !(li.firstChild instanceof HTMLElement && /^(UL|OL)$/.test(li.firstChild.tagName))) p.appendChild(li.firstChild);
        li.insertBefore(p, li.firstChild);
      }
    }
  }
  // GFM alerts: > [!NOTE] … → callouts.
  for (const q of [...doc.querySelectorAll('blockquote')]) {
    const first = q.querySelector('p');
    const m = first && /^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/i.exec(first.textContent ?? '');
    if (!first || !m) continue;
    const div = doc.createElement('div');
    div.setAttribute('data-callout', CALLOUT[m[1]!.toUpperCase()] ?? 'info');
    // Drop the marker from the first text node.
    const walker = doc.createTreeWalker(first, NodeFilter.SHOW_TEXT);
    const tn = walker.nextNode();
    if (tn) tn.textContent = (tn.textContent ?? '').replace(/^\s*\[![A-Za-z]+\]\s*/, '');
    if (!first.textContent?.trim()) first.remove();
    while (q.firstChild) div.appendChild(q.firstChild);
    if (!div.querySelector('p')) div.appendChild(doc.createElement('p'));
    q.replaceWith(div);
  }
  // Pictures only from our own uploads or the web (no data: URIs).
  for (const img of [...doc.querySelectorAll('img')]) if (!/^(https?:|\/api\/assets\/)/.test(img.getAttribute('src') ?? '')) img.remove();
  return doc.body.innerHTML;
}

/** Word file → HTML via the API. */
export async function docxToHtml(file: File): Promise<{ html: string; title: string }> {
  const fd = new FormData();
  fd.append('file', file);
  const r = await fetch('/api/docs/import-docx', { method: 'POST', body: fd, credentials: 'same-origin' });
  const body = (await r.json().catch(() => ({}))) as { html?: string; title?: string; message?: string };
  if (!r.ok || typeof body.html !== 'string') throw new Error(body.message ?? `HTTP ${r.status}`);
  return { html: normalizeImportedHtml(body.html), title: body.title ?? file.name };
}

export const isDocImport = (f: File) => /\.(md|markdown|docx)$/i.test(f.name);

/** A .md/.markdown/.docx file → HTML + a title. */
export async function fileToHtml(file: File): Promise<{ html: string; title: string }> {
  if (/\.docx$/i.test(file.name)) return docxToHtml(file);
  const md = await file.text();
  const h1 = /^#\s+(.+)$/m.exec(md)?.[1]?.trim();
  return { html: markdownToHtml(md), title: h1 || file.name.replace(/\.(md|markdown)$/i, '') };
}

/** Create a new doc holding the file's content; returns its id (the editor fills it on open). */
export async function importIntoNewDoc(file: File, folderId?: string | null): Promise<string> {
  const { html, title } = await fileToHtml(file);
  const { id } = await api.createDesign({ preset: 'doc', title: title.slice(0, 200), ...(folderId ? { folderId } : {}) });
  try { sessionStorage.setItem(KEY(id), html); } catch { /* storage full/blocked: the doc opens empty */ }
  return id;
}

/** HTML waiting to be placed into a freshly imported doc (consumed once). */
export function takePendingImport(id: string): string | null {
  try {
    const v = sessionStorage.getItem(KEY(id));
    if (v !== null) sessionStorage.removeItem(KEY(id));
    return v;
  } catch {
    return null;
  }
}
