import { Component, type ErrorInfo, type ReactNode } from 'react';
import { t } from '../i18n';

/**
 * Catches render errors below it (e.g. after an engine failure) and shows a recoverable message instead of a blank
 * page. The error is reported to the console. "Try again" remounts the subtree; "Reload" reloads the page, which
 * reopens the design from the server (edits already synced are kept).
 */
export class ErrorBoundary extends Component<{ children: ReactNode; label?: string }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error(`[${this.props.label ?? 'app'}] render error`, error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="empty" role="alert">
        <h2>{t('Something went wrong')}</h2>
        <p className="muted">{t('The editor hit an unexpected error. Your synced changes are safe; reload to continue.')}</p>
        <p style={{ display: 'flex', gap: 8, justifyContent: 'center' }}>
          <button className="btn primary" onClick={() => location.reload()}>{t('Reload')}</button>
          <button className="btn" onClick={() => this.setState({ error: null })}>{t('Try again')}</button>
        </p>
      </div>
    );
  }
}
