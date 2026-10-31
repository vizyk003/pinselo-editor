/** Brand controls (Brand Hub, A8): team settings that owners and admins use to keep designs on brand. */
import { useEffect, useState } from 'react';
import { ApiError, brandHub, type BrandControls } from '../api';
import { Modal, useToast } from '../components/ui';
import { t, tk } from '../i18n';

const ITEMS: Array<[keyof BrandControls, string, string]> = [
  ['colors', tk('Brand colours only'), tk('Colour pickers offer the team’s brand-kit colours only.')],
  ['fonts', tk('Brand fonts only'), tk('Font pickers offer the team’s brand-kit fonts only.')],
  ['requireTemplate', tk('Start from a brand template'), tk('Members and designers create team designs from the team’s brand templates.')],
  ['restrictUploads', tk('Restrict font uploads'), tk('Only owners and admins can upload brand fonts.')],
  ['requireApproval', tk('Require approval'), tk('Downloads, public links, website publishing and social scheduling wait until an owner or admin approves the design. Edits after approval need a new approval.')],
];

export function BrandControlsDialog({ workspaceId, onClose }: { workspaceId: string; onClose: () => void }) {
  const toast = useToast();
  const [data, setData] = useState<Awaited<ReturnType<typeof brandHub.controls>> | null>(null);
  useEffect(() => {
    void brandHub.controls(workspaceId).then(setData).catch(() => {});
  }, [workspaceId]);
  if (!data) return null;
  const set = async (k: keyof BrandControls, on: boolean) => {
    try {
      const r = await brandHub.setControls(workspaceId, { [k]: on });
      setData({ ...data, controls: r.controls });
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 402)) toast((e as Error).message);
    }
  };
  return (
    <Modal title={t('Brand controls')} onClose={onClose} size="sm">
      {!data.entitled.brandControls && <p className="muted" style={{ margin: 0 }}>{t('Brand controls and approvals are part of the Teams plan.')}</p>}
      {ITEMS.map(([k, label, help]) => (
        <label key={k} className="check" style={{ alignItems: 'flex-start' }}>
          <input type="checkbox" checked={Boolean(data.controls[k])} disabled={!data.canEdit} onChange={(e) => set(k, e.target.checked)} />
          <span><b>{t(label)}</b><br /><span className="faint" style={{ fontSize: 12 }}>{t(help)}</span></span>
        </label>
      ))}
      {!data.canEdit && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Only team owners and admins can change brand controls.')}</p>}
    </Modal>
  );
}
