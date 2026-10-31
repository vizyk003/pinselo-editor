/** AI panel: there is no built-in AI. People connect their own assistant (Claude, ChatGPT, Codex, …) over MCP. */
import { Sparkles } from 'lucide-react';
import { ConnectAi, useAiConnections } from '../components/ConnectAi';
import { CopyButton } from '../components/ui';
import { t } from '../i18n';
import { useDoc, useEd } from './context';
import { designPrompt } from './EditWithAi';

export function AiPanel() {
  const doc = useDoc();
  const { design } = useEd();
  const { apps, reload } = useAiConnections();
  const prompt = designPrompt(doc.meta.title, design.id, `${location.origin}/design/${design.id}`);
  return (
    <>
      <Sparkles size={28} className="faint" aria-hidden />
      <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('Connect Claude, ChatGPT, Codex or another MCP client to design with AI. The assistant works in your account and can only do what you allow.')}</p>
      <h4>{t('Prompt for your AI')}</h4>
      <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('Then ask it to work on “{title}”. You’ll see its edits live and can undo them.', { title: doc.meta.title })}</p>
      <div className="row" style={{ alignItems: 'flex-start' }}>
        <pre className="grow code-block" style={{ margin: 0, whiteSpace: 'pre-wrap', fontSize: 12 }}>{prompt}</pre>
        <CopyButton text={prompt} />
      </div>
      <ConnectAi connections={apps} reload={reload} />
    </>
  );
}
