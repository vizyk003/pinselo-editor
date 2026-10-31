/** Addresses inside translated text (review F1). */
import type { ReactNode } from 'react';

/** Fill a t() text's `{email}` with this, then hand the result to withEmail. */
export const EMAIL_SLOT = '\u0000';

/**
 * Translated text with its `{email}` (filled with EMAIL_SLOT) as an isolated left-to-right run: inside right-to-left
 * text (Arabic) an address's dots and @ would otherwise be reordered with the words and punctuation around it.
 */
export function withEmail(text: string, email: string): ReactNode {
  const [before, after = ''] = text.split(EMAIL_SLOT);
  return <>{before}<bdi dir="ltr">{email}</bdi>{after}</>;
}
