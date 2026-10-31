/**
 * The editor's "Edit with AI" entry point. With an assistant connected it copies a ready prompt that names this
 * design (title, id and link, so the assistant's MCP tools — get_design, edit_design … — find it) for the person to
 * paste into Claude, ChatGPT or Codex; without one it opens the same connect UI as the Dashboard.
 */
import { BRAND } from '@oc/config';
import { Copy } from 'lucide-react';
import { useState } from 'react';
import { ConnectAi, useAiConnections } from '../components/ConnectAi';
import { Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { useDoc, useEd } from './context';

/** The prompt that points an assistant at this design. */
export function designPrompt(title: string, id: string, url: string): string {
  return t('Using the {brand} connector, open my design “{title}” (design_id: {id}, {url}) with get_design and then: ', { brand: BRAND.name, title, id, url });
}

export function useEditWithAi() {
  const { design } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const { apps, reload } = useAiConnections();
  const [open, setOpen] = useState(false);
  const prompt = designPrompt(doc.meta.title, design.id, `${location.origin}/design/${design.id}`);
  const copyPrompt = async () => {
    try {
      await navigator.clipboard.writeText(prompt);
      const names = [...new Set((apps ?? []).map((a) => a.name))];
      toast(names.length ? t('Prompt copied. Paste it into {app} and say what to change.', { app: names.join(' / ') }) : t('Prompt copied'));
      return true;
    } catch {
      return false;
    }
  };
  const run = async () => {
    if (apps && apps.length > 0 && (await copyPrompt())) return;
    setOpen(true);
  };
  const element = open ? (
    <Modal title={t('Edit with AI')} size="lg" onClose={() => setOpen(false)}>
      <p className="muted" style={{ marginTop: 0 }}>{t('Connect an assistant, then paste this prompt into it and say what to change.')}</p>
      <div className="row" style={{ alignItems: 'flex-start', marginBottom: 16 }}>
        <pre className="grow code-block" style={{ margin: 0, whiteSpace: 'pre-wrap' }} aria-label={t('Prompt for your AI')}>{prompt}</pre>
        <button type="button" className="btn copy-btn" onClick={() => void copyPrompt()}><Copy size={14} aria-hidden /> {t('Copy prompt')}</button>
      </div>
      <ConnectAi connections={apps} reload={reload} />
    </Modal>
  ) : null;
  return { run: () => void run(), connected: !!apps?.length, element };
}
