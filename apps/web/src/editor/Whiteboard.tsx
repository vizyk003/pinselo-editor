/**
 * Whiteboard facilitation (WB-020..): sticky colours and clustering, sections, mind-map tidy-up, a shared
 * countdown timer, dot voting, emoji reactions, follow mode and spotlight.
 *
 *  - Timer and votes live in the design's Y.Doc (`board` / `votes` maps, via @oc/engine): real time and persisted.
 *  - Viewports, spotlight and reactions are ephemeral and travel over Hocuspocus awareness.
 *
 * `WhiteboardLayer` renders inside the canvas (section titles, sticky authors, vote dots, reactions);
 * `WhiteboardBar` is the floating facilitation toolbar plus the timer / vote widgets.
 */
import {
  castVote, clearVotes, observeBoard, readTimer, readVotes, readVoteSession, writeTimer, writeVoteSession, type TimerState, type VoteEntry, type VoteSession,
} from '@oc/engine';
import {
  formatClock, isMindNode, isSection, isSticky, makeTimer, mindNodeOf, sectionsOf, STICKY_COLORS, tallyVotes, timerAdd, timerPause, timerRemaining,
  timerReset, timerRunning, timerStart, topVoted, voteAllowance, voteScope, type Editor, type Viewport,
} from '@oc/engine';
import { worldBounds } from '@oc/engine';
import { childrenOf, newId, pageOf, sortedPages, type DesignDoc, type ElementId } from '@oc/engine';
import {
  ArrowDown, ArrowUp, ChevronDown, ChevronRight, Eye, LayoutGrid, Megaphone, Network, Pause, Play, Plus, RotateCcw, Smile, SquareDashed, StickyNote,
  Timer as TimerIcon, Trophy, Vote, Volume2, VolumeX, X, ZoomIn,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type * as Y from 'yjs';
import { useAuth } from '../App';
import { Avatar, PopoverButton, useToast } from '../components/ui';
import { t, tn } from '../i18n';
import { useEd, useEditorState, useView } from './context';

// ------------------------------------------------------------------ shared state hooks

interface AwUser { id: string; name: string; color: string }
interface AwState {
  user?: AwUser;
  cursor?: [number, number] | null;
  /** Scene rect the person is looking at. */
  view?: { x: number; y: number; w: number; h: number } | null;
  /** Spotlight start time while presenting their view to everyone. */
  spotlight?: number | null;
  reaction?: { id: string; emoji: string; x: number; y: number } | null;
}

/** Awareness states of every client (including this one), re-rendered on change. */
function useAwareness(): { self: number; states: Map<number, AwState> } {
  const { provider } = useEd();
  const aw = provider.awareness;
  const cache = useRef<{ v: number; snap: { self: number; states: Map<number, AwState> } }>({ v: -1, snap: { self: -1, states: new Map() } });
  const version = useRef(0);
  const subscribe = useCallback((cb: () => void) => {
    if (!aw) return () => {};
    const on = () => { version.current++; cb(); };
    aw.on('change', on);
    return () => aw.off('change', on);
  }, [aw]);
  return useSyncExternalStore(subscribe, () => {
    if (cache.current.v !== version.current) {
      cache.current = { v: version.current, snap: { self: aw?.clientID ?? -1, states: new Map(aw ? (aw.getStates() as Map<number, AwState>) : []) } };
    }
    return cache.current.snap;
  });
}

interface BoardSnap { timer: TimerState | null; session: VoteSession | null; votes: VoteEntry[] }
function useBoard(ydoc: Y.Doc): BoardSnap {
  const [v, setV] = useState(0);
  useEffect(() => observeBoard(ydoc, () => setV((x) => x + 1)), [ydoc]);
  return useMemo(() => {
    const session = readVoteSession(ydoc);
    return { timer: readTimer(ydoc), session, votes: session ? readVotes(ydoc, session.id) : [] };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ydoc, v]);
}

function useNow(active: boolean, ms = 250): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [active, ms]);
  return active ? now : Date.now();
}

/** Screen rect (canvas px) of an element. */
function screenRect(editor: Editor, doc: DesignDoc, vp: Viewport, id: ElementId) {
  const pid = pageOf(doc, id);
  if (!pid || !doc.elements[id]) return null;
  const [ox, oy] = editor.pageOrigin(pid);
  const b = worldBounds(doc, id);
  return { x: (b.x + ox - vp.x) * vp.zoom, y: (b.y + oy - vp.y) * vp.zoom, w: b.w * vp.zoom, h: b.h * vp.zoom };
}

function chime() {
  try {
    const ctx = new AudioContext();
    [0, 0.25, 0.5].forEach((d, i) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.frequency.value = [880, 660, 990][i]!;
      g.gain.setValueAtTime(0.0001, ctx.currentTime + d);
      g.gain.exponentialRampToValueAtTime(0.25, ctx.currentTime + d + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + d + 0.22);
      o.connect(g).connect(ctx.destination);
      o.start(ctx.currentTime + d);
      o.stop(ctx.currentTime + d + 0.25);
    });
    setTimeout(() => void ctx.close(), 1200);
  } catch { /* audio unavailable */ }
}

const CSS = `
@keyframes wb-float { 0% { transform: translate(-50%, 0) scale(.6); opacity: 0 } 12% { transform: translate(-50%, -12px) scale(1.15); opacity: 1 } 100% { transform: translate(-50%, -140px) scale(1); opacity: 0 } }
.wb-layer { position: absolute; inset: 0; pointer-events: none; overflow: hidden; }
.wb-reaction { position: absolute; font-size: 34px; animation: wb-float 1.9s ease-out forwards; text-shadow: 0 2px 6px rgba(0,0,0,.2); }
.wb-section-title { position: absolute; display: flex; align-items: center; gap: 4px; font-family: Inter, sans-serif; font-weight: 600; color: #334155; white-space: nowrap; pointer-events: auto; cursor: default; }
.wb-section-title button { pointer-events: auto; border: 0; background: transparent; color: inherit; cursor: pointer; padding: 2px; display: grid; place-items: center; border-radius: 4px; }
.wb-section-title button:hover { background: rgba(15,23,42,.08); }
.wb-author { position: absolute; font: 500 10px Inter, sans-serif; color: rgba(15,23,42,.55); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.wb-vote-target { position: absolute; pointer-events: auto; cursor: copy; border-radius: 6px; outline: 2px dashed rgba(201,51,22,.45); outline-offset: 2px; }
.wb-vote-target:hover { background: rgba(201,51,22,.08); }
.wb-badge { position: absolute; min-width: 22px; height: 22px; padding: 0 6px; border-radius: 11px; background: #c93316; color: #fff; font: 700 12px/22px Inter, sans-serif; text-align: center; box-shadow: 0 2px 6px rgba(0,0,0,.25); transform: translate(-50%, -50%); }
.wb-dots { position: absolute; display: flex; gap: 3px; transform: translateY(-50%); }
.wb-dots i { width: 10px; height: 10px; border-radius: 50%; background: #ec4899; border: 1.5px solid #fff; box-shadow: 0 1px 2px rgba(0,0,0,.3); }
.wb-highlight { position: absolute; border-radius: 10px; border: 3px solid #f59e0b; box-shadow: 0 0 0 6px rgba(245,158,11,.25); }
.wb-rank { position: absolute; transform: translate(-50%, -50%); width: 26px; height: 26px; border-radius: 50%; color: #fff; font: 800 13px/26px Inter, sans-serif; text-align: center; box-shadow: 0 2px 6px rgba(0,0,0,.3); }
/* Placed in the stage's canvas row (an absolute grid child takes its grid area as containing block), so it sits
   below the element toolbar however many rows that wraps to (WB-01), not at a fixed 62px from the stage top. */
.wb-bar { position: absolute; grid-row: 2; grid-column: 1; top: 10px; left: 50%; transform: translateX(-50%); z-index: 6; display: flex; align-items: center; gap: 2px; padding: 4px; background: var(--surface); border: 1px solid var(--border); border-radius: 12px; box-shadow: 0 6px 20px rgba(0,0,0,.12); max-width: calc(100% - 24px); overflow-x: auto; scrollbar-width: none; }
.wb-bar .sep { width: 1px; height: 22px; background: var(--border); margin: 0 4px; flex: none; }
.wb-widgets { position: absolute; grid-row: 2; grid-column: 1; top: 10px; right: 12px; z-index: 6; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; pointer-events: none; }
.wb-card { pointer-events: auto; background: var(--surface); border: 1px solid var(--border); border-radius: 12px; box-shadow: 0 6px 20px rgba(0,0,0,.12); padding: 10px 12px; min-width: 190px; }
.wb-clock { font: 700 28px/1.1 Inter, ui-monospace, monospace; font-variant-numeric: tabular-nums; letter-spacing: .02em; }
.wb-clock.done { color: #dc2626; }
.wb-swatches { display: grid; grid-template-columns: repeat(4, 28px); gap: 6px; padding: 8px; }
.wb-swatches button { width: 28px; height: 28px; border-radius: 6px; border: 1px solid rgba(0,0,0,.12); cursor: pointer; }
.wb-pop { padding: 10px; display: flex; flex-direction: column; gap: 8px; min-width: 240px; }
.wb-pop label { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.wb-pop .row { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
.wb-list { display: flex; flex-direction: column; gap: 2px; max-height: 320px; overflow: auto; }
.wb-list > div { display: flex; align-items: center; gap: 4px; padding: 2px 4px; border-radius: 6px; }
.wb-list > div:hover { background: var(--hover, rgba(0,0,0,.04)); }
.wb-list .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; cursor: pointer; }
.wb-people { display: flex; align-items: center; }
.wb-people button { border: 0; background: none; padding: 0; margin-left: -4px; cursor: pointer; border-radius: 50%; }
.wb-people button.following { outline: 2px solid #c93316; outline-offset: 1px; }
.wb-banner { pointer-events: auto; display: flex; align-items: center; gap: 8px; background: #c93316; color: #fff; border-radius: 10px; padding: 6px 10px; font-size: 13px; box-shadow: 0 6px 20px rgba(0,0,0,.18); }
.wb-banner button { color: #fff; }
`;

const REACTIONS = ['👍', '❤️', '😂', '🎉', '🤔', '👏', '🔥', '💡'];
const RANK_COLORS = ['#f59e0b', '#94a3b8', '#b45309'];

// ------------------------------------------------------------------ in-canvas layer

export function WhiteboardLayer() {
  const { editor, readOnly } = useEd();
  const state = useEditorState();
  const view = useView();
  const toast = useToast();
  const { user } = useAuth();
  const board = useBoard(editor.store.ydoc);
  const aw = useAwareness();
  const vp = state.viewport;
  const ydoc = editor.store.ydoc;

  // Floating reactions: every new reaction id from any client floats up once.
  const seen = useRef(new Set<string>());
  const [floating, setFloating] = useState<Array<{ id: string; emoji: string; x: number; y: number }>>([]);
  useEffect(() => {
    const fresh: Array<{ id: string; emoji: string; x: number; y: number }> = [];
    for (const s of aw.states.values()) {
      const r = s.reaction;
      if (r && !seen.current.has(r.id)) {
        seen.current.add(r.id);
        fresh.push(r);
      }
    }
    if (!fresh.length) return;
    setFloating((f) => [...f, ...fresh]);
    const ids = new Set(fresh.map((r) => r.id));
    const timer = setTimeout(() => setFloating((f) => f.filter((r) => !ids.has(r.id))), 2000);
    return () => clearTimeout(timer);
  }, [aw]);

  const tops = useMemo(() => sortedPages(view).flatMap((p) => childrenOf(view, p.id)), [view]);
  const session = board.session;
  const tally = useMemo(() => (session ? tallyVotes(session, board.votes) : []), [session, board.votes]);
  const mine = user?.id ?? '';
  const left = session ? voteAllowance(session, board.votes, mine) : 0;
  const myOn = (el: ElementId) => board.votes.filter((v) => v.voter === mine && v.element === el).reduce((s, v) => s + v.n, 0);
  const vote = (el: ElementId, delta: number) => {
    if (!session || session.status !== 'open' || readOnly || !user) return;
    if (delta > 0 && left <= 0) {
      toast(t('You have used all your votes'));
      return;
    }
    if (delta < 0 && myOn(el) <= 0) return;
    castVote(ydoc, session.id, { id: user.id, ...(session.anonymous ? {} : { name: user.name }) }, el, delta);
  };
  const fs = Math.max(11, Math.min(26, 18 * vp.zoom));

  return (
    <div className="wb-layer">
      <style>{CSS}</style>
      {/* section titles */}
      {tops.filter(isSection).map((s) => {
        const r = screenRect(editor, view, vp, s.id);
        if (!r || r.w < 30) return null;
        const sec = s.board!.section!;
        return (
          <div key={s.id} className="wb-section-title" style={{ left: r.x + 10, top: r.y + 8, fontSize: fs, maxWidth: Math.max(40, r.w - 20) }}
            onDoubleClick={(e) => {
              e.stopPropagation();
              if (readOnly) return;
              const next = prompt(t('Section name'), sec.title);
              if (next?.trim()) editor.renameSection(s.id, next.trim());
            }}>
            {!readOnly && (
              <button title={sec.collapsed ? t('Expand section') : t('Collapse section')} onPointerDown={(e) => e.stopPropagation()} onClick={() => editor.toggleSection(s.id)}>
                {sec.collapsed ? <ChevronRight size={fs} /> : <ChevronDown size={fs} />}
              </button>
            )}
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{sec.title}</span>
            {sec.collapsed && <span style={{ fontWeight: 400, opacity: 0.6, fontSize: fs * 0.75 }}>{tn(sec.collapsed.hidden.length, '{n} item', '{n} items')}</span>}
          </div>
        );
      })}
      {/* sticky authors */}
      {tops.filter((e) => e.board?.sticky?.author && e.visible !== false).map((s) => {
        const r = screenRect(editor, view, vp, s.id);
        if (!r || r.h < 70) return null;
        return <div key={s.id} className="wb-author" style={{ left: r.x + 8, top: r.y + r.h - 18, maxWidth: r.w - 16 }}>{s.board!.sticky!.author!.name}</div>;
      })}
      {/* votes */}
      {session && tally.map((tl) => {
        const el = view.elements[tl.element];
        if (!el || el.deleted || el.visible === false) return null;
        const r = screenRect(editor, view, vp, tl.element);
        if (!r) return null;
        const minePlaced = myOn(tl.element);
        const who = session.anonymous ? '' : tl.voters.map((v) => `${v.name ?? t('Someone')} ×${v.n}`).join(', ');
        return (
          <div key={tl.element}>
            {session.status === 'open' && !readOnly && (
              <div className="wb-vote-target" style={{ left: r.x, top: r.y, width: r.w, height: r.h }}
                title={t('Click to vote · right-click or ⌥-click to remove')}
                onPointerDown={(e) => e.stopPropagation()}
                onDoubleClick={(e) => e.stopPropagation()}
                onClick={(e) => vote(tl.element, e.altKey ? -1 : 1)}
                onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); vote(tl.element, -1); }} />
            )}
            {minePlaced > 0 && (
              <div className="wb-dots" style={{ left: r.x + 6, top: r.y + r.h - 8 }}>
                {Array.from({ length: Math.min(minePlaced, 10) }, (_, i) => <i key={i} />)}
              </div>
            )}
            {tl.count > 0 && <div className="wb-badge" style={{ left: r.x + r.w, top: r.y }} title={who}>{tl.count}</div>}
          </div>
        );
      })}
      {session?.highlight?.map((id, i) => {
        const r = screenRect(editor, view, vp, id);
        if (!r || view.elements[id]?.deleted) return null;
        return (
          <div key={id}>
            <div className="wb-highlight" style={{ left: r.x - 8, top: r.y - 8, width: r.w + 16, height: r.h + 16 }} />
            <div className="wb-rank" style={{ left: r.x - 8, top: r.y - 8, background: RANK_COLORS[Math.min(i, 2)] }}>{i + 1}</div>
          </div>
        );
      })}
      {/* reactions */}
      {floating.map((r) => {
        const x = (r.x - vp.x) * vp.zoom;
        const y = (r.y - vp.y) * vp.zoom;
        return <span key={r.id} className="wb-reaction" style={{ left: x, top: y - 20 }}>{r.emoji}</span>;
      })}
    </div>
  );
}

// ------------------------------------------------------------------ floating bar + widgets

export function WhiteboardBar() {
  const { editor, provider, readOnly } = useEd();
  const state = useEditorState();
  const view = useView();
  const toast = useToast();
  const { user } = useAuth();
  const aw = useAwareness();
  const board = useBoard(editor.store.ydoc);
  const ydoc = editor.store.ydoc;
  const awareness = provider.awareness;

  useEffect(() => {
    editor.user = user ? { id: user.id, name: user.name } : null;
  }, [editor, user]);

  // ---- broadcast my viewport (scene rect) for followers
  useEffect(() => {
    if (!awareness) return undefined;
    let last = '';
    let timer = 0;
    const push = () => {
      timer = 0;
      const v = editor.state.viewport;
      const rect = { x: Math.round(v.x), y: Math.round(v.y), w: Math.round(v.width / v.zoom), h: Math.round(v.height / v.zoom) };
      const key = JSON.stringify(rect);
      if (key !== last) {
        last = key;
        awareness.setLocalStateField('view', rect);
      }
    };
    push();
    const off = editor.subscribe(() => {
      if (!timer) timer = window.setTimeout(push, 80);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [editor, awareness]);

  // ---- follow mode / spotlight
  const [followId, setFollowId] = useState<number | null>(null);
  const [dismissedSpot, setDismissedSpot] = useState<number>(0);
  const mySpot = aw.states.get(aw.self)?.spotlight ?? null;
  const spotlighter = useMemo(() => {
    let best: [number, AwState] | null = null;
    for (const [id, s] of aw.states) if (id !== aw.self && s.spotlight && s.user && (!best || s.spotlight > best[1].spotlight!)) best = [id, s];
    return best;
  }, [aw]);
  const spotActive = spotlighter && spotlighter[1].spotlight! > dismissedSpot && !mySpot ? spotlighter : null;
  const target = spotActive ? spotActive[0] : followId;
  const targetState = target !== null ? aw.states.get(target) : undefined;
  const applied = useRef<string>('');
  useEffect(() => {
    if (target === null) return;
    if (!targetState) {
      if (followId !== null && target === followId) setFollowId(null);
      return;
    }
    const r = targetState.view;
    if (!r) return;
    const v = editor.state.viewport;
    const zoom = Math.min(v.width / r.w, v.height / r.h);
    const next = { ...v, zoom, x: r.x + r.w / 2 - v.width / 2 / zoom, y: r.y + r.h / 2 - v.height / 2 / zoom };
    applied.current = `${next.x.toFixed(1)}:${next.y.toFixed(1)}:${next.zoom.toFixed(4)}`;
    editor.setViewport(next);
  }, [editor, target, targetState, followId]);
  // Panning or zooming yourself stops following (spotlight: dismisses it).
  useEffect(() => {
    if (target === null) return undefined;
    return editor.subscribe(() => {
      const v = editor.state.viewport;
      const key = `${v.x.toFixed(1)}:${v.y.toFixed(1)}:${v.zoom.toFixed(4)}`;
      if (applied.current && key !== applied.current && !editor.state.gesturing) {
        applied.current = '';
        if (spotActive) setDismissedSpot(spotActive[1].spotlight!);
        setFollowId(null);
      }
    });
  }, [editor, target, spotActive]);

  const people = useMemo(() => {
    const out: Array<{ clientId: number; user: AwUser }> = [];
    const ids = new Set<string>();
    for (const [id, s] of aw.states) {
      if (id === aw.self || !s.user || ids.has(`${s.user.id}:${id}`)) continue;
      ids.add(`${s.user.id}:${id}`);
      out.push({ clientId: id, user: s.user });
    }
    return out;
  }, [aw]);

  // ---- reactions
  const react = (emoji: string) => {
    if (!awareness) return;
    const me = aw.states.get(aw.self);
    const v = editor.state.viewport;
    const at = me?.cursor ?? [v.x + v.width / v.zoom / 2, v.y + v.height / v.zoom / 2];
    awareness.setLocalStateField('reaction', { id: newId(), emoji, x: at[0], y: at[1] });
  };

  // ---- selection-derived
  const sel = state.selection.map((id) => view.elements[id]).filter((e) => e && !e.deleted);
  const stickies = sel.filter((e) => isSticky(e));
  const mindSel = state.selection.length === 1 ? mindNodeOf(view, state.selection[0]!) : null;
  const sections = sectionsOf(view, state.activePageId);

  return (
    <>
      <style>{CSS}</style>
      <div className="wb-bar" onPointerDown={(e) => e.stopPropagation()} role="toolbar" aria-label={t('Whiteboard tools')}>
        {!readOnly && (
          <>
            <PopoverButton className="btn ghost sm" title={stickies.length ? t('Sticky colour') : t('Add sticky note')} label={<><StickyNote size={16} /> {t('Sticky')}</>} content={(close) => (
              <div className="wb-swatches">
                {STICKY_COLORS.map((c) => (
                  <button key={c} style={{ background: c }} title={c} aria-label={c} onClick={() => {
                    close();
                    if (stickies.length) editor.setStickyColor(c, stickies.map((s) => s!.id));
                    else {
                      const id = editor.addSticky(c, { size: 240 });
                      if (id) editor.editGroupText(id);
                    }
                  }} />
                ))}
              </div>
            )} />
            {stickies.length > 1 && (
              <button className="btn ghost sm" title={t('Group selected stickies by colour into tidy grids')} onClick={() => editor.clusterStickies(stickies.map((s) => s!.id))}>
                <LayoutGrid size={16} /> {t('Cluster')}
              </button>
            )}
            <SectionsMenu sections={sections} />
            <button className="btn ghost sm" disabled={!mindSel || !isMindNode(mindSel)} onClick={() => mindSel && editor.layoutMindMap(mindSel.id)}
              title={t('Tidy the mind map (select a node). Tab adds a child, Enter adds a sibling.')}>
              <Network size={16} /> {t('Tidy map')}
            </button>
            <span className="sep" />
          </>
        )}
        <TimerMenu timer={board.timer} readOnly={readOnly} />
        <VoteMenu board={board} readOnly={readOnly} />
        <PopoverButton className="btn ghost sm icon" title={t('React')} label={<Smile size={16} />} content={(close) => (
          <div className="row" style={{ padding: 6, gap: 2 }}>
            {REACTIONS.map((e) => <button key={e} className="btn ghost sm icon" style={{ fontSize: 20 }} onClick={() => { react(e); close(); }}>{e}</button>)}
          </div>
        )} />
        <span className="sep" />
        <button className={`btn ghost sm ${mySpot ? 'active' : ''}`} title={mySpot ? t('Stop spotlight') : t('Spotlight: bring everyone to your view')}
          onClick={() => {
            awareness?.setLocalStateField('spotlight', mySpot ? null : Date.now());
            if (!mySpot) toast(t('Everyone is now following your view'));
          }}>
          <Megaphone size={16} /> {mySpot ? t('Stop spotlight') : t('Spotlight')}
        </button>
        {people.length > 0 && (
          <div className="wb-people" style={{ marginLeft: 8 }}>
            {people.slice(0, 6).map((p) => (
              <button key={p.clientId} className={target === p.clientId ? 'following' : ''} title={target === p.clientId ? t('Stop following {name}', { name: p.user.name }) : t('Follow {name}', { name: p.user.name })}
                onClick={() => setFollowId(followId === p.clientId ? null : p.clientId)}>
                <Avatar name={p.user.name} color={p.user.color} size={26} />
              </button>
            ))}
          </div>
        )}
      </div>
      <div className="wb-widgets">
        {target !== null && targetState?.user && (
          <div className="wb-banner">
            <Eye size={15} />
            {spotActive ? t('{name} is spotlighting their view', { name: targetState.user.name }) : t('Following {name}', { name: targetState.user.name })}
            <button className="btn ghost sm icon" title={t('Stop following')} onClick={() => {
              if (spotActive) setDismissedSpot(spotActive[1].spotlight!);
              setFollowId(null);
            }}><X size={14} /></button>
          </div>
        )}
        {board.timer && <TimerCard timer={board.timer} readOnly={readOnly} ydoc={ydoc} />}
        {board.session && <VoteCard board={board} readOnly={readOnly} />}
      </div>
    </>
  );
}

// ------------------------------------------------------------------ sections

function SectionsMenu({ sections }: { sections: ReturnType<typeof sectionsOf> }) {
  const { editor } = useEd();
  return (
    <PopoverButton className="btn ghost sm" title={t('Sections')} label={<><SquareDashed size={16} /> {t('Sections')}</>} content={(close) => (
      <div className="wb-pop">
        <button className="btn sm" onClick={() => {
          close();
          editor.createSection(t('Section {n}', { n: sections.length + 1 }));
        }}><Plus size={14} /> {editor.state.selection.length ? t('Add section around selection') : t('Add section')}</button>
        {sections.length ? (
          <div className="wb-list">
            {sections.map((s, i) => {
              const sec = s.board!.section!;
              return (
                <div key={s.id}>
                  <span className="name" title={t('Zoom to section')} onClick={() => { editor.zoomToElement(s.id); editor.select([s.id]); }}>{sec.title}</span>
                  <button className="btn ghost sm icon" title={t('Zoom to section')} onClick={() => editor.zoomToElement(s.id)}><ZoomIn size={14} /></button>
                  <button className="btn ghost sm icon" title={sec.collapsed ? t('Expand section') : t('Collapse section')} onClick={() => editor.toggleSection(s.id)}>
                    {sec.collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                  </button>
                  <button className="btn ghost sm icon" title={t('Move up')} disabled={i === 0} onClick={() => editor.moveSection(s.id, -1)}><ArrowUp size={14} /></button>
                  <button className="btn ghost sm icon" title={t('Move down')} disabled={i === sections.length - 1} onClick={() => editor.moveSection(s.id, 1)}><ArrowDown size={14} /></button>
                  <button className="btn ghost sm icon" title={t('Rename')} onClick={() => {
                    const next = prompt(t('Section name'), sec.title);
                    if (next?.trim()) editor.renameSection(s.id, next.trim());
                  }}>✎</button>
                </div>
              );
            })}
          </div>
        ) : <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Sections group ideas: anything inside one moves with it.')}</p>}
      </div>
    )} />
  );
}

// ------------------------------------------------------------------ timer

function TimerMenu({ timer, readOnly }: { timer: TimerState | null; readOnly: boolean }) {
  const { editor } = useEd();
  const ydoc = editor.store.ydoc;
  const [mins, setMins] = useState(5);
  const [sound, setSound] = useState(timer?.sound ?? true);
  const start = (m: number) => {
    const next = timerStart({ ...makeTimer(Math.round(m * 60_000), sound), by: editor.user?.name }, Date.now());
    writeTimer(ydoc, next);
  };
  return (
    <PopoverButton className="btn ghost sm" title={t('Timer')} label={<><TimerIcon size={16} /> {t('Timer')}</>} content={(close) => (
      <div className="wb-pop">
        {readOnly ? <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Only editors can run the timer.')}</p> : (
          <>
            <div className="row">
              {[1, 3, 5, 10, 15].map((m) => <button key={m} className="btn sm" onClick={() => { start(m); close(); }}>{t('{n} min', { n: m })}</button>)}
            </div>
            <div className="row">
              <input className="input" type="number" min={0.5} max={180} step={0.5} value={mins} onChange={(e) => setMins(Math.max(0.5, Math.min(180, +e.target.value || 1)))} style={{ width: 80 }} aria-label={t('Minutes')} />
              <button className="btn sm primary" onClick={() => { start(mins); close(); }}><Play size={14} /> {t('Start')}</button>
            </div>
            <label><input type="checkbox" checked={sound} onChange={(e) => setSound(e.target.checked)} /> {t('Play a sound when time is up')}</label>
            {timer && <button className="btn sm" onClick={() => { writeTimer(ydoc, null); close(); }}><X size={14} /> {t('Remove timer')}</button>}
          </>
        )}
      </div>
    )} />
  );
}

function TimerCard({ timer, readOnly, ydoc }: { timer: TimerState; readOnly: boolean; ydoc: Y.Doc }) {
  const running = timer.endsAt !== null;
  const now = useNow(running);
  const left = timerRemaining(timer, now);
  const done = running && left <= 0;
  const rang = useRef<number | null>(null);
  useEffect(() => {
    if (done && timer.sound && rang.current !== timer.endsAt) {
      rang.current = timer.endsAt;
      // Only ring if we saw it reach zero recently (not when opening a board whose timer ended long ago).
      if (now - (timer.endsAt ?? 0) < 3000) chime();
    }
  }, [done, timer.sound, timer.endsAt, now]);
  const set = (next: TimerState | null) => writeTimer(ydoc, next);
  const n = Date.now();
  return (
    <div className="wb-card" role="timer" aria-live="off">
      <div className="row" style={{ justifyContent: 'space-between', gap: 8 }}>
        <span className={`wb-clock ${done ? 'done' : ''}`}>{done ? t("Time's up") : formatClock(left)}</span>
        {timer.sound ? <Volume2 size={14} className="faint" /> : <VolumeX size={14} className="faint" />}
      </div>
      {!readOnly && (
        <div className="row" style={{ gap: 4, marginTop: 6 }}>
          {timerRunning(timer, n) ? (
            <button className="btn ghost sm icon" title={t('Pause')} onClick={() => set(timerPause(timer, Date.now()))}><Pause size={15} /></button>
          ) : (
            <button className="btn ghost sm icon" title={t('Start')} onClick={() => set(timerStart(done ? timerReset(timer) : timer, Date.now()))}><Play size={15} /></button>
          )}
          <button className="btn ghost sm icon" title={t('Reset')} onClick={() => set(timerReset(timer))}><RotateCcw size={15} /></button>
          <button className="btn ghost sm" title={t('Add a minute')} onClick={() => set(done ? timerStart({ ...timer, endsAt: null, remainingMs: 60_000 }, Date.now()) : timerAdd(timer, 60_000, Date.now()))}>{t('+1 min')}</button>
          <button className="btn ghost sm icon" title={timer.sound ? t('Mute') : t('Unmute')} onClick={() => set({ ...timer, sound: !timer.sound })}>{timer.sound ? <VolumeX size={15} /> : <Volume2 size={15} />}</button>
          <button className="btn ghost sm icon" title={t('Remove timer')} onClick={() => set(null)}><X size={15} /></button>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ voting

function VoteMenu({ board, readOnly }: { board: BoardSnap; readOnly: boolean }) {
  const { editor } = useEd();
  const { user } = useAuth();
  const state = useEditorState();
  const [per, setPer] = useState(3);
  const [anonymous, setAnonymous] = useState(true);
  const [scope, setScope] = useState<'selection' | 'all'>(state.selection.length ? 'selection' : 'all');
  const ydoc = editor.store.ydoc;
  const open = board.session?.status === 'open';
  const start = () => {
    const ids = voteScope(editor.doc, editor.state.activePageId, scope === 'selection' ? editor.state.selection : []);
    if (!ids.length || !user) return false;
    if (board.session) clearVotes(ydoc, board.session.id);
    writeVoteSession(ydoc, {
      id: newId(), perPerson: per, anonymous, scope: ids, status: 'open',
      startedBy: { id: user.id, name: user.name }, startedAt: Date.now(),
    });
    editor.clearSelection();
    return true;
  };
  return (
    <PopoverButton className={`btn ghost sm ${open ? 'active' : ''}`} title={t('Vote')} label={<><Vote size={16} /> {t('Vote')}</>} content={(close) => (
      <div className="wb-pop">
        {readOnly ? <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Only editors can start a vote.')}</p> : open ? (
          <>
            <p style={{ margin: 0, fontSize: 13 }}>{t('A vote is in progress.')}</p>
            <button className="btn sm primary" onClick={() => { writeVoteSession(ydoc, { ...board.session!, status: 'ended', endedAt: Date.now() }); close(); }}>{t('End vote')}</button>
          </>
        ) : (
          <>
            <label>{t('Votes per person')}
              <input className="input" type="number" min={1} max={20} value={per} onChange={(e) => setPer(Math.max(1, Math.min(20, Math.round(+e.target.value || 1))))} style={{ width: 64 }} />
            </label>
            <label><input type="checkbox" checked={anonymous} onChange={(e) => setAnonymous(e.target.checked)} /> {t('Anonymous voting')}</label>
            <label><input type="radio" name="wb-scope" checked={scope === 'selection'} disabled={!state.selection.length} onChange={() => setScope('selection')} /> {t('Selected elements ({n})', { n: state.selection.length })}</label>
            <label><input type="radio" name="wb-scope" checked={scope === 'all'} onChange={() => setScope('all')} /> {t('All stickies and shapes on the board')}</label>
            <button className="btn sm primary" onClick={() => { if (start()) close(); }}><Vote size={14} /> {t('Start vote')}</button>
          </>
        )}
      </div>
    )} />
  );
}

function VoteCard({ board, readOnly }: { board: BoardSnap; readOnly: boolean }) {
  const { editor } = useEd();
  const { user } = useAuth();
  const view = useView();
  const session = board.session!;
  const ydoc = editor.store.ydoc;
  const tally = useMemo(() => tallyVotes(session, board.votes), [session, board.votes]);
  const left = voteAllowance(session, board.votes, user?.id ?? '');
  const voters = new Set(board.votes.map((v) => v.voter)).size;
  const label = (id: ElementId) => {
    const el = view.elements[id];
    if (!el) return t('(deleted)');
    const texts = [el, ...childrenOf(view, id)].filter((e) => e.type === 'text');
    const txt = texts.map((e) => (e.type === 'text' ? e.body.paragraphs.map((p) => p.runs.map((r) => r.text).join('')).join(' ') : '')).join(' ').trim();
    return txt || el.name || t('Untitled element');
  };
  if (session.status === 'open') {
    return (
      <div className="wb-card">
        <div style={{ fontWeight: 600, fontSize: 13 }}>{t('Voting in progress')}</div>
        <div className="faint" style={{ fontSize: 12, margin: '4px 0 8px' }}>
          {readOnly ? t('{n} people have voted', { n: voters }) : t('{left} of {total} votes left · click items to vote', { left, total: session.perPerson })}
        </div>
        {!readOnly && <button className="btn sm primary" onClick={() => writeVoteSession(ydoc, { ...session, status: 'ended', endedAt: Date.now() })}>{t('End vote')}</button>}
      </div>
    );
  }
  const ranked = tally.filter((x) => x.count > 0);
  return (
    <div className="wb-card" style={{ width: 260 }}>
      <div className="row" style={{ justifyContent: 'space-between' }}>
        <strong style={{ fontSize: 13 }}><Trophy size={14} /> {t('Vote results')}</strong>
        {!readOnly && <button className="btn ghost sm icon" title={t('Clear results')} onClick={() => { clearVotes(ydoc, session.id); writeVoteSession(ydoc, null); }}><X size={14} /></button>}
      </div>
      <div className="wb-list" style={{ margin: '6px 0' }}>
        {ranked.length ? ranked.map((x, i) => (
          <div key={x.element}>
            <span style={{ width: 18, fontWeight: 700, color: RANK_COLORS[i] ?? 'inherit' }}>{i + 1}</span>
            <span className="name" onClick={() => { editor.zoomToElement(x.element, 120); editor.select([x.element]); }}
              title={session.anonymous ? undefined : x.voters.map((v) => v.name ?? t('Someone')).join(', ')}>{label(x.element)}</span>
            <span className="badge">{x.count}</span>
          </div>
        )) : <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('No votes were cast.')}</p>}
      </div>
      {!readOnly && ranked.length > 0 && (
        <button className="btn sm" onClick={() => {
          const top = topVoted(tally, 3);
          writeVoteSession(ydoc, { ...session, highlight: top });
          editor.select(top.filter((id) => editor.doc.elements[id] && !editor.doc.elements[id]!.deleted));
        }}><Trophy size={14} /> {t('Highlight top 3')}</button>
      )}
    </div>
  );
}
