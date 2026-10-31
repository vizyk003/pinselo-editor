import type { HocuspocusProvider } from '@hocuspocus/provider';
import type { Editor, EditorState } from '@oc/engine';
import type { DesignDoc } from '@oc/engine';
import { createContext, useContext, useSyncExternalStore } from 'react';
import type { DesignSummary, Role } from '../api';
import type { Engine } from '../lib/engine';

export interface Presence {
  clientId: number;
  /** `ai`: an AI assistant connected through the AI connector, working for `onBehalfOf`. */
  user: { id: string; name: string; color: string; ai?: boolean; onBehalfOf?: string };
  cursor?: [number, number] | null;
  selection?: string[];
  pageId?: string;
}

export type Drawer = 'comments' | 'versions' | 'layers' | 'a11y' | null;

export interface EditorCtx {
  editor: Editor;
  engine: Engine;
  provider: HocuspocusProvider;
  design: DesignSummary & { role: Role };
  role: Role;
  readOnly: boolean;
  drawer: Drawer;
  setDrawer: (d: Drawer) => void;
  openDialog: (d: 'share' | 'export' | 'resize' | 'shortcuts' | 'find' | 'grid' | 'publish' | 'bulk' | 'site' | null) => void;
  setPresenting: (on: boolean) => void;
}

export const Ctx = createContext<EditorCtx | null>(null);

export function useEd(): EditorCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error('editor context missing');
  return c;
}

/** Re-render on any editor state or document change. */
export function useEditorState(): EditorState {
  const { editor } = useEd();
  return useSyncExternalStore(
    (cb) => editor.subscribe(cb),
    () => editor.state,
  );
}

export function useDoc(): DesignDoc {
  const { editor } = useEd();
  return useSyncExternalStore(
    (cb) => editor.subscribe(cb),
    () => editor.doc,
  );
}

/** The document including in-flight gesture previews. */
export function useView(): DesignDoc {
  const { editor } = useEd();
  // `view` builds a new object when overrides exist; cache per editor version.
  return useSyncExternalStore(
    (cb) => editor.subscribe(cb),
    () => viewCache(editor),
  );
}

const cache = new WeakMap<Editor, { version: number; view: DesignDoc }>();
function viewCache(editor: Editor): DesignDoc {
  const c = cache.get(editor);
  if (c && c.version === editor.version) return c.view;
  const view = editor.view;
  cache.set(editor, { version: editor.version, view });
  return view;
}
