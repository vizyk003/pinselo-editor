import { Download } from 'lucide-react';
import { PROVIDER_COLOR, PROVIDER_SHORT, type ProviderId, type SocialAccount } from './api';

/** Avatar with a small network mark (or just the network mark when there is no avatar). */
export function AccountBadge({ account, size = 28 }: { account: Pick<SocialAccount, 'provider' | 'avatarUrl' | 'displayName'>; size?: number }) {
  return (
    <span className="acct-badge" style={{ width: size, height: size }}>
      {account.avatarUrl
        ? <img src={account.avatarUrl} alt="" referrerPolicy="no-referrer" />
        : <ProviderMark provider={account.provider} size={size} />}
      {account.avatarUrl && <span className="mark"><ProviderMark provider={account.provider} size={Math.round(size * 0.5)} /></span>}
    </span>
  );
}

export function ProviderMark({ provider, size = 24 }: { provider: ProviderId; size?: number }) {
  return (
    <span className="provider-mark" aria-hidden style={{ width: size, height: size, background: PROVIDER_COLOR[provider], fontSize: Math.max(8, size * 0.42) }}>
      {provider === 'local' ? <Download size={Math.round(size * 0.55)} /> : PROVIDER_SHORT[provider]}
    </span>
  );
}
