/** A team's brand kits (Brand Hub, A8), listed from the team page: logo, colours, fonts and what each kit holds. */
import { Plus, Star } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { ApiError, assetUrl, brandHub, type Kit } from '../api';
import { Modal, useToast } from '../components/ui';
import { brandStore } from '../editor/brandStore';
import { t, tn } from '../i18n';

export function TeamBrandKitsDialog({ workspaceId, teamName, onClose }: { workspaceId: string; teamName: string; onClose: () => void }) {
  const toast = useToast();
  const [kits, setKits] = useState<Kit[] | null>(null);
  const [canEdit, setCanEdit] = useState(false);
  const [busy, setBusy] = useState(false);
  const load = useCallback(
    () =>
      brandHub.kits().then((r) => {
        setKits(r.kits.filter((k) => k.workspaceId === workspaceId));
        setCanEdit(Boolean(r.workspaces.find((w) => w.id === workspaceId)?.canEdit));
        void brandStore.loadKits(true);
      }).catch(() => setKits([])),
    [workspaceId],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const act = (p: Promise<unknown>) => {
    setBusy(true);
    return p.then(load).catch((e) => {
      if (!(e instanceof ApiError && e.status === 402)) toast(e instanceof ApiError ? e.message : t('That didn’t work.'));
    }).finally(() => setBusy(false));
  };

  return (
    <Modal title={t('{team} · brand kits', { team: teamName })} onClose={onClose} size="lg">
      <div className="col" style={{ gap: 12 }}>
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {t('Everyone in the team can use these kits in team designs. To change a kit, open a design and use the Brand panel.')}
        </p>
        {canEdit && (
          <div className="row">
            <button className="btn" disabled={busy} onClick={() => void act(brandHub.createKit({ name: t('New brand kit'), workspaceId }).then(({ id }) => brandStore.setActiveKit(id)))}>
              <Plus size={16} /> {t('Create a kit for {team}', { team: teamName })}
            </button>
          </div>
        )}
        {!kits && <div className="faint" style={{ fontSize: 13 }}>{t('Loading…')}</div>}
        {kits && !kits.length && <div className="faint" style={{ fontSize: 13 }}>{t('This team has no brand kits yet.')}</div>}
        <div className="list">
          {kits?.map((k) => {
            const fonts = [...new Set(Object.values(k.fonts).filter(Boolean))];
            const logo = k.logos[0];
            return (
              <div className="item" key={k.id} style={{ alignItems: 'flex-start' }}>
                <span aria-hidden style={{ flex: 'none', width: 72, height: 48, borderRadius: 6, background: 'var(--surface-3)', display: 'grid', placeItems: 'center', overflow: 'hidden' }}>
                  {logo && <img src={assetUrl(logo)} alt="" style={{ maxWidth: '86%', maxHeight: '80%', objectFit: 'contain' }} draggable={false} />}
                </span>
                <div className="main col" style={{ gap: 6, minWidth: 0 }}>
                  <div className="row" style={{ gap: 6 }}>
                    <b>{k.name}</b>
                    {k.isDefault && <span className="pill">{t('Default')}</span>}
                  </div>
                  {!!k.colors.length && (
                    <div className="row" style={{ gap: 4, flexWrap: 'wrap' }} aria-label={t('Colours')}>
                      {k.colors.map((c) => (
                        <span key={c.hex + c.name} title={c.name ? `${c.name} · ${c.hex}` : c.hex} style={{ width: 18, height: 18, borderRadius: 4, background: c.hex, border: '1px solid var(--border)' }} />
                      ))}
                    </div>
                  )}
                  <div className="sub">
                    {[
                      fonts.length ? fonts.join(', ') : null,
                      k.logos.length ? tn(k.logos.length, '{n} logo', '{n} logos') : null,
                      k.graphics.length ? tn(k.graphics.length, '{n} graphic', '{n} graphics') : null,
                      k.photos.length ? tn(k.photos.length, '{n} photo', '{n} photos') : null,
                    ].filter(Boolean).join(' · ')}
                  </div>
                </div>
                {canEdit && !k.isDefault && (
                  <button className="btn ghost sm" disabled={busy} onClick={() => void act(brandHub.makeDefault(k.id))}>
                    <Star size={14} /> {t('Make default kit')}
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </Modal>
  );
}
