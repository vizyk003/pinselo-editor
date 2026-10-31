import { fmtRelative, t } from '../i18n';
import { Check, Copy, LayoutGrid, List as ListIcon, Search, X } from 'lucide-react';
import { createContext, Fragment, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type InputHTMLAttributes, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { messageMs } from '../lib/appearance';

/** Modal widths: sm 480px (short forms), md 560px (default), lg 720px, xl 980px (browsers like Create a design). */
export type ModalSize = 'sm' | 'md' | 'lg' | 'xl';
export function Modal({ title, onClose, children, size = 'md' }: { title: string; onClose: () => void; children: ReactNode; size?: ModalSize }) {
  const box = useRef<HTMLDivElement>(null);
  // Captured while rendering, before children commit: React runs `autoFocus` in the layout phase, so by the
  // time an effect runs the focused element is already inside the dialog.
  const [opener] = useState(() => (typeof document === 'undefined' ? null : (document.activeElement as HTMLElement | null)));
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [onClose]);
  // Keyboard focus stays inside the dialog while it's open and returns to whatever opened it.
  useEffect(() => {
    const el = box.current;
    if (el && !el.contains(document.activeElement)) (el.querySelector<HTMLElement>('[autofocus], input, select, textarea') ?? el).focus();
    const trap = (e: KeyboardEvent) => {
      if (e.key !== 'Tab' || !el) return;
      const items = [...el.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')].filter((n) => n.offsetParent !== null);
      if (!items.length) return;
      const first = items[0]!, last = items[items.length - 1]!;
      if (!el.contains(document.activeElement)) { e.preventDefault(); first.focus(); }
      else if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    };
    document.addEventListener('keydown', trap);
    return () => { document.removeEventListener('keydown', trap); if (opener?.isConnected) opener.focus(); };
  }, [opener]);
  return createPortal(
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${size}`} ref={box} tabIndex={-1} role="dialog" aria-modal="true" aria-label={title}>
        <header>
          <h2>{title}</h2>
          <button className="btn ghost icon sm" onClick={onClose} aria-label={t('Close')}>
            <X size={18} />
          </button>
        </header>
        <div className="body">{children}</div>
      </div>
    </div>,
    document.body,
  );
}

/**
 * A popover anchored below (or above, or beside) an element; closes on outside click / Escape. `point` opens at a
 * 1px anchor such as the cursor: down and to the right, flipping up or left so the menu's edge sits at the point.
 */
export function Popover({ anchor, onClose, children, placement = 'below', menu, minTop = 8, className }: {
  anchor: HTMLElement | null; onClose: () => void; children: ReactNode; placement?: 'below' | 'above' | 'right' | 'point';
  /** Menu keyboard behaviour: focus the first item on open, Up/Down/Home/End move between items, Tab closes. */
  menu?: boolean;
  /** Keep the popover's top edge below this (a header it must not cover). */
  minTop?: number;
  /** Extra classes, e.g. the `sm` / `md` width of the popover. */
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number; maxHeight?: number } | null>(null);
  const place = useCallback(() => {
    const el = ref.current;
    if (!anchor || !el) return;
    const a = anchor.getBoundingClientRect();
    const w = el.offsetWidth;
    // The content's own height, also while a max-height clamps the box (else clamping and measuring would alternate).
    const h = el.scrollHeight + (el.offsetHeight - el.clientHeight);
    const W = window.innerWidth, H = window.innerHeight;
    let left = placement === 'right' ? a.right + 8 : a.left;
    if (placement === 'point' && left + w > W - 8) left = a.left - w;
    left = Math.max(8, Math.min(left, W - w - 8));
    // Taller than the room there is (a long menu on a short screen): pin it to the top and let it scroll, so every item stays reachable.
    if (h > H - minTop - 8) return setPos({ left, top: minTop, maxHeight: H - minTop - 8 });
    let top = placement === 'below' ? a.bottom + 6 : placement === 'above' ? a.top - h - 6 : a.top;
    if (top + h > H - 8) top = placement === 'point' ? a.top - h : placement === 'right' ? a.bottom - h : a.top - h - 6;
    setPos({ left, top: Math.max(minTop, Math.min(top, H - h - 8)) });
  }, [anchor, placement, minTop]);
  useLayoutEffect(() => {
    place();
    // Content that loads or changes size after opening is placed again (it would otherwise overflow from its first spot).
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => place());
    ro.observe(el);
    return () => ro.disconnect();
  }, [place]);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node) || anchor?.contains(e.target as Node)) return;
      onClose();
    };
    const key = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('mousedown', down, true);
    window.addEventListener('keydown', key);
    return () => {
      window.removeEventListener('mousedown', down, true);
      window.removeEventListener('keydown', key);
    };
  }, [anchor, onClose]);
  const placed = pos !== null;
  useEffect(() => {
    if (menu && placed) ref.current?.querySelector<HTMLElement>('button:not([disabled])')?.focus();
  }, [menu, placed]);
  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (!menu) return;
    if (e.key === 'Tab') { anchor?.focus(); onClose(); return; }
    const items = [...(ref.current?.querySelectorAll<HTMLElement>('button:not([disabled])') ?? [])];
    if (!items.length) return;
    const i = items.indexOf(document.activeElement as HTMLElement);
    const next = e.key === 'ArrowDown' ? (i + 1) % items.length : e.key === 'ArrowUp' ? (i - 1 + items.length) % items.length
      : e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : -1;
    if (next < 0) return;
    e.preventDefault();
    items[next]!.focus();
  };
  return createPortal(
    <div ref={ref} className={`popover${className ? ` ${className}` : ''}`} onKeyDown={onKeyDown}
      style={pos ? { left: pos.left, top: pos.top, ...(pos.maxHeight ? { maxHeight: pos.maxHeight, overflowY: 'auto' } : {}) } : { visibility: 'hidden', left: 0, top: 0 }}>
      {children}
    </div>,
    document.body,
  );
}

/** Button that toggles a popover. */
export function PopoverButton({ label, className, children, content, title, placement, menu, anchorClosest, popoverClassName }: {
  label?: ReactNode; className?: string; children?: ReactNode; title?: string; placement?: 'below' | 'above' | 'right';
  content: (close: () => void) => ReactNode;
  /** The content is a menu (role="menu" items): keyboard focus moves into it and returns to the button on close. */
  menu?: boolean;
  /** Anchor the popover to the button's closest ancestor matching this selector (a small badge on a larger tile). */
  anchorClosest?: string;
  /** Classes for the popover box, e.g. its `sm` (260px) or `md` (320px) width. */
  popoverClassName?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => {
    setOpen(false);
    // Back to the button unless focus already went somewhere else (an outside click).
    const a = document.activeElement;
    if (menu && (!a || a === document.body || a.closest('.popover'))) ref.current?.focus();
  }, [menu]);
  return (
    <>
      <button ref={ref} className={className ?? 'btn ghost sm'} title={title} aria-label={title} aria-expanded={open} aria-haspopup={menu ? 'menu' : undefined} onClick={() => setOpen((o) => !o)}>
        {label ?? children}
      </button>
      {open && (
        <Popover anchor={(anchorClosest && ref.current?.closest<HTMLElement>(anchorClosest)) || ref.current} onClose={close} placement={placement} menu={menu} className={popoverClassName}>
          {content(close)}
        </Popover>
      )}
    </>
  );
}

/**
 * A menu row: a fixed 16px icon column, the label and an optional shortcut. `checked` makes it a toggle
 * (menuitemcheckbox) whose icon column shows a check while it's on.
 */
export function MenuItem({ icon, label, kbd, onClick, danger, disabled, checked }: {
  icon?: ReactNode; label: string; kbd?: string; onClick: () => void; danger?: boolean; disabled?: boolean; checked?: boolean;
}) {
  const toggle = checked !== undefined;
  return (
    <button className={danger ? 'danger' : undefined} onClick={onClick} disabled={disabled} style={disabled ? { opacity: 0.45 } : undefined}
      role={toggle ? 'menuitemcheckbox' : 'menuitem'} aria-checked={toggle ? checked : undefined}>
      <span className="menu-ic" aria-hidden>{toggle && !icon ? (checked ? <Check size={16} /> : null) : icon}</span>
      <span className="grow">{label}</span>
      {toggle && icon && checked && <Check size={16} className="menu-check" aria-hidden />}
      {kbd && <span className="kbd">{kbd}</span>}
    </button>
  );
}

// ---------------------------------------------------------------- search field

/**
 * The search box used everywhere (dashboard top bars, editor panels, dialogs): a 36px input with a magnifier and,
 * when `onClear` is given, a clear button once something is typed. Width comes from the layout (`.topbar` 280px).
 */
export function SearchField({ value, onChange, placeholder, label, onClear, className, inputClassName, autoFocus, onKeyDown, inputProps }: {
  value: string; onChange: (v: string) => void; placeholder?: string;
  /** Accessible name (defaults to the placeholder). */
  label?: string;
  onClear?: () => void; className?: string; inputClassName?: string; autoFocus?: boolean;
  onKeyDown?: (e: ReactKeyboardEvent<HTMLInputElement>) => void;
  inputProps?: Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'placeholder'>;
}) {
  return (
    <label className={`search-field${className ? ` ${className}` : ''}`}>
      <Search size={16} aria-hidden />
      <input className={`input${inputClassName ? ` ${inputClassName}` : ''}`} type="search" value={value} placeholder={placeholder} aria-label={label ?? placeholder}
        autoFocus={autoFocus} onChange={(e) => onChange(e.target.value)} onKeyDown={onKeyDown} {...inputProps} />
      {onClear && value && <button type="button" className="search-clear" aria-label={t('Clear search')} onClick={onClear}><X size={14} /></button>}
    </label>
  );
}

// ---------------------------------------------------------------- grid / list toggle

/** The grid/list toggle of every dashboard list (Projects, Starred, Shared, Team, Uploads, Trash): a small segmented control. */
export function ViewToggle({ mode, onChange }: { mode: 'grid' | 'list'; onChange: (v: 'grid' | 'list') => void }) {
  return (
    <div className="segmented sm icons" role="group" aria-label={t('View')}>
      <button type="button" className={mode === 'grid' ? 'on' : ''} aria-pressed={mode === 'grid'} title={t('Grid view')} aria-label={t('Grid view')} onClick={() => onChange('grid')}><LayoutGrid size={16} /></button>
      <button type="button" className={mode === 'list' ? 'on' : ''} aria-pressed={mode === 'list'} title={t('List view')} aria-label={t('List view')} onClick={() => onChange('list')}><ListIcon size={16} /></button>
    </div>
  );
}

// ---------------------------------------------------------------- copy button

/** Copies `text` and confirms with a toast: icon and "Copy" label at the 36px height of the input or code beside it. */
export function CopyButton({ text, className }: { text: string; className?: string }) {
  const toast = useToast();
  return (
    <button type="button" className={`btn copy-btn${className ? ` ${className}` : ''}`} onClick={() => { void navigator.clipboard.writeText(text); toast(t('Copied')); }}>
      <Copy size={14} /> {t('Copy')}
    </button>
  );
}

// ---------------------------------------------------------------- empty state

/** What a list shows when there is nothing in it: an icon, an optional title, a line of help and an optional action. */
export function EmptyState({ icon, title, children, action, className }: { icon?: ReactNode; title?: ReactNode; children?: ReactNode; action?: ReactNode; className?: string }) {
  return (
    <div className={`empty-state${className ? ` ${className}` : ''}`}>
      {icon && <span className="empty-state-ic" aria-hidden>{icon}</span>}
      {title && <p className="empty-state-title">{title}</p>}
      {children && <p className="empty-state-line">{children}</p>}
      {action && <div className="empty-state-action">{action}</div>}
    </div>
  );
}

// ---------------------------------------------------------------- page header

/**
 * The header of every dashboard page: a sticky bar with the page title (24px, on the 28px page gutter) and the
 * page's actions after it. `title` may be breadcrumbs; pass `titleAs="div"` when they bring their own heading.
 */
export function PageHeader({ title, children, className, titleAs = 'h1' }: { title: ReactNode; children?: ReactNode; className?: string; titleAs?: 'h1' | 'div' }) {
  const Title = titleAs;
  return (
    <div className={`topbar page-header${className ? ` ${className}` : ''}`}>
      <Title className="page-title">{title}</Title>
      {children}
    </div>
  );
}

// ---------------------------------------------------------------- horizontal scrollers

/**
 * Ref for a horizontally scrolling row (tiles, tabs, page thumbnails): adds the `scroll-fade` class and keeps
 * `data-fade` at "start", "end", "start end" or "none", so the CSS fades whichever edge has more content past it.
 */
export function useScrollFade<T extends HTMLElement = HTMLDivElement>(): (el: T | null) => void {
  const cleanup = useRef<(() => void) | null>(null);
  return useCallback((el: T | null) => {
    cleanup.current?.();
    cleanup.current = null;
    if (!el) return;
    el.classList.add('scroll-fade');
    const update = () => {
      const max = el.scrollWidth - el.clientWidth;
      const x = Math.abs(el.scrollLeft); // negative in right-to-left layouts
      const fade = [x > 1 && 'start', x < max - 1 && 'end'].filter(Boolean).join(' ') || 'none';
      if (el.dataset.fade !== fade) el.dataset.fade = fade;
    };
    update();
    el.addEventListener('scroll', update, { passive: true });
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    ro?.observe(el);
    // Content that arrives later (thumbnails, translated labels) changes the scroll width without resizing the row.
    const mo = typeof MutationObserver === 'undefined' ? null : new MutationObserver(update);
    mo?.observe(el, { childList: true, subtree: true });
    cleanup.current = () => {
      el.removeEventListener('scroll', update);
      ro?.disconnect();
      mo?.disconnect();
    };
  }, []);
}

/** A horizontally scrolling row (template cards, category tiles) that fades whichever edge has more past it. */
export function ScrollRow({ className, children }: { className: string; children: ReactNode }) {
  const ref = useScrollFade();
  return <div className={className} ref={ref}>{children}</div>;
}

// ---------------------------------------------------------------- toasts

const ToastCtx = createContext<(msg: string) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastHost({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Array<{ id: number; msg: string }>>([]);
  const push = useCallback((msg: string) => {
    const id = Math.random();
    setItems((s) => [...s, { id, msg }]);
    setTimeout(() => setItems((s) => s.filter((x) => x.id !== id)), messageMs());
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toast-host" role="status" aria-live="polite">
        {items.map((x) => (
          <div key={x.id} className="toast">
            {x.msg}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

export function initials(name: string): string {
  return name.split(/\s+/).filter(Boolean).slice(0, 2).map((p) => p[0]!.toUpperCase()).join('') || '?';
}

/** Initials colour for an avatar: white, or near-black on light colours where white would fall under 4.5:1. */
export function avatarText(bg: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(bg.trim());
  if (!m) return '#fff';
  const n = parseInt(m[1]!, 16);
  const lin = (v: number) => { const c = v / 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
  const lum = 0.2126 * lin(n >> 16) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255);
  return 1.05 / (lum + 0.05) >= 4.5 ? '#fff' : '#17171c';
}

export function Avatar({ name, color, size = 30 }: { name: string; color: string; size?: number }) {
  return (
    <span className="avatar" title={name} style={{ background: color, color: avatarText(color), width: size, height: size, fontSize: size * 0.4 }}>
      {initials(name)}
    </span>
  );
}

export function timeAgo(iso: string): string {
  return fmtRelative(iso);
}

/**
 * Interpolate React nodes into an already-translated sentence: `interp(t('Sent to {email}.'), { email: <b>{email}</b> })`.
 * Keeps whole sentences as one translation key when they contain inline elements.
 */
export function interp(text: string, parts: Record<string, ReactNode>): ReactNode {
  return text.split(/(\{\w+\})/g).map((seg, i) => {
    const m = /^\{(\w+)\}$/.exec(seg);
    return m && m[1]! in parts ? <Fragment key={i}>{parts[m[1]!]}</Fragment> : seg;
  });
}
