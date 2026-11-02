/** Listens for 402 responses anywhere in the app and explains the fair-use limit (storage) that was reached. */
import { useEffect, useState } from 'react';
import { t } from '../i18n';
import { Modal } from './ui';

export function LimitHost() {
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => {
    const on = (e: Event) => setMessage((e as CustomEvent<{ message?: string }>).detail?.message ?? '');
    window.addEventListener('oc:limit', on);
    return () => window.removeEventListener('oc:limit', on);
  }, []);
  if (message === null) return null;
  return (
    <Modal title={t('Limit reached')} onClose={() => setMessage(null)} size="sm">
      <p style={{ margin: 0 }}>{message}</p>
      <div className="row"><span className="grow" /><button className="btn primary" onClick={() => setMessage(null)}>{t('Close')}</button></div>
    </Modal>
  );
}
