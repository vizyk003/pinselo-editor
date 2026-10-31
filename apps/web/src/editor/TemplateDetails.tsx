/**
 * Editor Templates panel details popover (g25): a larger thumbnail, size, type, page count and tags, with Apply —
 * the panel no longer applies a template on the first click.
 */
import { formatDims } from '@oc/engine';
import { Crown } from 'lucide-react';
import { useEffect, useState } from 'react';
import { plansEnabled, templateThumbUrl, type TemplateSummary } from '../api';
import { Popover } from '../components/ui';
import { t, tn } from '../i18n';
import { fetchTemplatePreview, presetOf, type TemplatePreviewData } from '../lib/templates';

export function TemplateDetailsPopover({ tpl, anchor, onClose, onApply }: { tpl: TemplateSummary; anchor: HTMLElement; onClose: () => void; onApply: () => void }) {
  const [details, setDetails] = useState<TemplatePreviewData | null>(null);
  useEffect(() => {
    let live = true;
    fetchTemplatePreview(tpl.id).then((d) => live && setDetails(d)).catch(() => {});
    return () => { live = false; };
  }, [tpl.id]);
  const preset = presetOf(tpl.preset);
  const unit = (details?.template.unit ?? preset?.unit ?? 'px') as Parameters<typeof formatDims>[2];
  const pages = details?.template.pages ?? 0;
  return (
    <Popover anchor={anchor} onClose={onClose} placement="right">
      <div className="tpl-pop" role="dialog" aria-label={tpl.title}>
        <div className="tpl-pop-img">
          <img src={templateThumbUrl(tpl.id)} alt="" />
          {tpl.premium && plansEnabled() && <span className="premium-badge"><Crown size={11} /> {t('PRO')}</span>}
          {tpl.team && <span className="premium-badge team">{t('TEAM')}</span>}
        </div>
        <strong>{tpl.title}</strong>
        <div className="faint">
          {t(tpl.category)} · {preset ? `${t(preset.label)} · ` : ''}{formatDims(tpl.width, tpl.height, unit)}
          {pages > 1 && <> · {tn(pages, '{n} page', '{n} pages')}</>}
        </div>
        {details?.template.description && <div className="muted">{details.template.description}</div>}
        {(tpl.tags ?? []).length > 0 && <div className="faint tpl-pop-tags">{tpl.tags.slice(0, 6).map((x) => `#${x}`).join(' ')}</div>}
        <button className="btn primary" onClick={onApply} autoFocus>
          {pages > 1 ? t('Add {n} pages', { n: pages }) : t('Apply template')}
        </button>
      </div>
    </Popover>
  );
}
