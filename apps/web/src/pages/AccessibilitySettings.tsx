/** Settings → Accessibility (plan 11 g03): theme, contrast, motion, shortcuts and message timing, saved on this device. */
import { Monitor, Moon, Sun } from 'lucide-react';
import { setAppearance, useAppearance, type ThemePref } from '../lib/appearance';
import { t, tk } from '../i18n';
import { Panel } from './Settings';

const THEMES: Array<{ id: ThemePref; label: string; Icon: typeof Sun }> = [
  { id: 'system', label: tk('Match my system'), Icon: Monitor }, { id: 'light', label: tk('Light'), Icon: Sun }, { id: 'dark', label: tk('Dark'), Icon: Moon },
];

export function AccessibilityTab() {
  const a = useAppearance();
  return (
    <>
      <Panel title={t('Theme')} sub={t('Saved on this device.')}>
        <div className="row theme-choice" role="radiogroup" aria-label={t('Theme')} style={{ gap: 8, flexWrap: 'wrap' }}>
          {THEMES.map(({ id, label, Icon }) => (
            <button key={id} type="button" role="radio" aria-checked={a.theme === id} className={`btn ${a.theme === id ? 'active' : ''}`} onClick={() => setAppearance({ theme: id })}>
              <Icon size={16} aria-hidden /> {t(label)}
            </button>
          ))}
        </div>
      </Panel>
      <Panel title={t('Accessibility')} sub={t('These settings apply to this browser on this device.')}>
        <div className="col" style={{ gap: 12 }}>
          <label className="check">
            <input type="checkbox" checked={a.contrast} onChange={(e) => setAppearance({ contrast: e.target.checked })} />
            <span><strong>{t('More contrast')}</strong><br /><span className="muted">{t('Darker text and borders, and a stronger outline around the focused control.')}</span></span>
          </label>
          <label className="check">
            <input type="checkbox" checked={a.motion === 'reduce'} onChange={(e) => setAppearance({ motion: e.target.checked ? 'reduce' : 'system' })} />
            <span><strong>{t('Reduce motion')}</strong><br /><span className="muted">{t('Turn off animations and transitions in the app, even when your system allows them.')}</span></span>
          </label>
          <label className="check">
            <input type="checkbox" checked={a.shortcutsNeedModifier} onChange={(e) => setAppearance({ shortcutsNeedModifier: e.target.checked })} />
            <span><strong>{t('Shortcuts need a modifier key')}</strong><br /><span className="muted">{t('Single-letter editor shortcuts (T, R, C, L) only work with ⌘, Ctrl or Alt held, so stray key presses do nothing.')}</span></span>
          </label>
          <label className="check">
            <input type="checkbox" checked={a.longerMessages} onChange={(e) => setAppearance({ longerMessages: e.target.checked })} />
            <span><strong>{t('Keep messages on screen longer')}</strong><br /><span className="muted">{t('Confirmation messages stay for 8 seconds instead of 3.')}</span></span>
          </label>
        </div>
      </Panel>
    </>
  );
}
