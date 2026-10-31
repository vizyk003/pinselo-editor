/**
 * Overlay chrome for vector editing (DS-100): the path being drawn with the pen (nodes, handles, rubber band to
 * the cursor) and the nodes and handles of a path whose points are being edited. Nodes and handles carry
 * `data-pathnode="s:i:part"`, which the canvas turns into the editor's `pathNode` hit target.
 */
import type { ElementId, PageId, PathNode, SubPath, Vec2 } from '@oc/engine';
import { useEd, useEditorState, useView } from './context';

type Props = {
  S: (p: Vec2) => [number, number];
  origin: (pageId: string | undefined) => Vec2;
  pageOfEl: (id: ElementId) => PageId | undefined;
  coarse: boolean;
};

/** SVG path data through nodes (cubic where a handle is set). */
function pathD(sub: SubPath, at: (p: Vec2) => [number, number]): string {
  const n = sub.nodes.length;
  if (!n) return '';
  const f = (p: Vec2) => at(p).join(' ');
  let d = `M${f(sub.nodes[0]!.p)}`;
  const segs = sub.closed ? n : n - 1;
  for (let k = 0; k < segs; k++) {
    const a = sub.nodes[k]!;
    const b = sub.nodes[(k + 1) % n]!;
    d += a.out || b.in ? ` C${f(a.out ?? a.p)} ${f(b.in ?? b.p)} ${f(b.p)}` : ` L${f(b.p)}`;
  }
  return sub.closed ? `${d} Z` : d;
}

function Handles({ node, at, r, ids }: { node: PathNode; at: (p: Vec2) => [number, number]; r: number; ids?: [string, string] }) {
  const a = at(node.p);
  return (
    <>
      {([['in', node.in], ['out', node.out]] as const).map(([part, h], k) => {
        if (!h) return null;
        const q = at(h);
        return (
          <g key={part}>
            <line x1={a[0]} y1={a[1]} x2={q[0]} y2={q[1]} stroke="var(--accent)" strokeWidth={1} />
            <circle className={ids ? 'hit' : undefined} data-pathnode={ids?.[k]} cx={q[0]} cy={q[1]} r={r} fill="#fff" stroke="var(--accent)" strokeWidth={1.5} style={ids ? { cursor: 'move' } : undefined} />
          </g>
        );
      })}
    </>
  );
}

export function VectorOverlay({ S, origin, pageOfEl, coarse }: Props) {
  const { editor } = useEd();
  const state = useEditorState();
  useView();
  const node = coarse ? 9 : 4.5;
  const handle = coarse ? 8 : 4;
  const out: React.ReactNode[] = [];

  const pen = state.pen;
  if (pen && state.tool === 'pen') {
    const o = origin(pen.pageId);
    const at = (p: Vec2) => S([p[0] + o[0], p[1] + o[1]]);
    const last = pen.nodes[pen.nodes.length - 1];
    out.push(<path key="pen" d={pathD({ nodes: pen.nodes, closed: false }, at)} fill="none" stroke="var(--accent)" strokeWidth={1.5} />);
    if (last && pen.cursor && !pen.dragging) {
      const a = at(last.p);
      const c = at(pen.cursor);
      const h = last.out ? at(last.out) : a;
      out.push(<path key="rubber" d={`M${a.join(' ')} Q${h.join(' ')} ${c.join(' ')}`} fill="none" stroke="var(--accent)" strokeWidth={1} strokeDasharray="4 3" />);
    }
    pen.nodes.forEach((n, i) => {
      const p = at(n.p);
      // The first node is where a click closes the path.
      out.push(i === 0 && pen.nodes.length > 1
        ? <circle key={`n${i}`} cx={p[0]} cy={p[1]} r={node + 2} fill="var(--surface)" stroke="var(--accent)" strokeWidth={2} />
        : <rect key={`n${i}`} x={p[0] - node} y={p[1] - node} width={node * 2} height={node * 2} fill="#fff" stroke="var(--accent)" strokeWidth={1.5} />);
    });
    if (last) out.push(<Handles key="h" node={last} at={at} r={handle} />);
    if (pen.cursor) {
      const c = at(pen.cursor);
      out.push(
        <g key="cursor" stroke="var(--accent)" strokeWidth={1.5}>
          <line x1={c[0] - 7} y1={c[1]} x2={c[0] + 7} y2={c[1]} />
          <line x1={c[0]} y1={c[1] - 7} x2={c[0]} y2={c[1] + 7} />
        </g>,
      );
    }
  }

  const pe = state.pathEdit ? editor.pathEditView() : null;
  if (pe) {
    const o = origin(pageOfEl(pe.id));
    const at = (p: Vec2) => S([p[0] + o[0], p[1] + o[1]]);
    const selected = new Set(pe.selected.map((r) => `${r.s}:${r.i}`));
    const total = pe.subpaths.reduce((k, sp) => k + sp.nodes.length, 0);
    pe.subpaths.forEach((sp, s) => {
      out.push(<path key={`o${s}`} d={pathD(sp, at)} fill="none" stroke="var(--accent)" strokeWidth={1} />);
      const n = sp.nodes.length;
      sp.nodes.forEach((nd, i) => {
        // Handles of the selected nodes and their neighbours (all of them on small paths).
        const near = total <= 24 || [i, (i + 1) % n, (i + n - 1) % n].some((j) => selected.has(`${s}:${j}`));
        if (near) out.push(<Handles key={`h${s}:${i}`} node={nd} at={at} r={handle} ids={[`${s}:${i}:in`, `${s}:${i}:out`]} />);
      });
      sp.nodes.forEach((nd, i) => {
        const p = at(nd.p);
        const on = selected.has(`${s}:${i}`);
        const shape = nd.smooth
          ? <circle cx={p[0]} cy={p[1]} r={node + 0.5} />
          : <rect x={p[0] - node} y={p[1] - node} width={node * 2} height={node * 2} />;
        out.push(
          <g key={`n${s}:${i}`} className="hit" data-pathnode={`${s}:${i}:anchor`} fill={on ? 'var(--accent)' : '#fff'} stroke="var(--accent)" strokeWidth={1.5} style={{ cursor: 'move' }}>
            {shape}
          </g>,
        );
      });
    });
  }
  return out.length ? <g>{out}</g> : null;
}
