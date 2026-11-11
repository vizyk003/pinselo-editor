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

/** Instagram's glyph on its gradient, or Facebook's "f" on its blue: the posting channels of outside publishers. */
export function ChannelLogo({ channel, size = 18 }: { channel: 'instagram' | 'facebook'; size?: number }) {
  if (channel === 'instagram') {
    const gid = `ig-grad-${size}`;
    return (
      <svg className="channel-logo" width={size} height={size} viewBox="0 0 24 24" role="img" aria-label="Instagram">
        <defs>
          <radialGradient id={gid} cx="30%" cy="107%" r="150%">
            <stop offset="0" stopColor="#fdf497" /><stop offset="0.05" stopColor="#fdf497" /><stop offset="0.45" stopColor="#fd5949" />
            <stop offset="0.6" stopColor="#d6249f" /><stop offset="0.9" stopColor="#285AEB" />
          </radialGradient>
        </defs>
        <rect width="24" height="24" rx="6" fill={`url(#${gid})`} />
        <rect x="5" y="5" width="14" height="14" rx="4.2" fill="none" stroke="#fff" strokeWidth="1.8" />
        <circle cx="12" cy="12" r="3.3" fill="none" stroke="#fff" strokeWidth="1.8" />
        <circle cx="16.3" cy="7.7" r="1.05" fill="#fff" />
      </svg>
    );
  }
  return (
    <svg className="channel-logo" width={size} height={size} viewBox="0 0 24 24" role="img" aria-label="Facebook">
      <circle cx="12" cy="12" r="12" fill="#0866FF" />
      <path fill="#fff" d="M13.4 24v-8.6h2.8l.5-3.3h-3.3V10c0-.95.3-1.6 1.66-1.6h1.77V5.47A23 23 0 0 0 14.25 5.3c-2.55 0-4.3 1.56-4.3 4.42v2.4H7.1v3.3h2.85V24z" />
    </svg>
  );
}
