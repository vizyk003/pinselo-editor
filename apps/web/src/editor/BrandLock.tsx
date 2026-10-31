/** Brand-template locks (A8): mark elements as locked for team members (position & size, content, style). */
import type { Element } from '@oc/engine';
import { ShieldCheck } from 'lucide-react';
import { PopoverButton } from '../components/ui';
import { t } from '../i18n';
import { useBrandHub } from './brandStore';
import { useEd } from './context';

export function BrandLockButton({ sel }: { sel: Element[] }) {
  const { editor } = useEd();
  const hub = useBrandHub();
  const locks = sel.map((e) => (e.lock?.by === 'brand' ? e.lock : null));
  const any = locks.some(Boolean);
  // Offered on team designs (where templates are published) or when the selection already carries a brand lock.
  if (!any && !hub.design?.workspaceId) return null;
  if (!editor.brandAdmin) {
    return any ? <span className="btn ghost sm icon" title={t('Locked by the brand template')} aria-label={t('Locked by the brand template')}><ShieldCheck size={16} /></span> : null;
  }
  const has = (k: 'move' | 'content' | 'style') => locks.some((l) => l?.[k]);
  const toggle = (k: 'position' | 'content' | 'style', on: boolean) => {
    const cur = { position: has('move'), content: has('content'), style: has('style') };
    editor.setBrandLock({ ...cur, [k]: on });
  };
  return (
    <PopoverButton title={t('Brand template lock')} label={<ShieldCheck size={16} className={any ? 'accent' : undefined} />} content={() => (
      <div className="col pop-sm" style={{ gap: 6 }}>
        <b>{t('Lock for team members')}</b>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Applies when this design is published as a brand template. Owners and admins can still edit.')}</p>
        <label className="check"><input type="checkbox" checked={has('move')} onChange={(e) => toggle('position', e.target.checked)} /> {t('Position and size')}</label>
        <label className="check"><input type="checkbox" checked={has('content')} onChange={(e) => toggle('content', e.target.checked)} /> {t('Content (text, image)')}</label>
        <label className="check"><input type="checkbox" checked={has('style')} onChange={(e) => toggle('style', e.target.checked)} /> {t('Style (colours, fonts, effects)')}</label>
      </div>
    )} />
  );
}
