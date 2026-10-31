/**
 * "Continue as {email}?" and the email-conflict notice (review F1): the address is an isolated left-to-right run, so
 * in Arabic its dots and @ are not reordered with the surrounding words and punctuation.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { EMAIL_SLOT, withEmail } from '../apps/web/src/auth/bidi';

const WEB = resolve(import.meta.dirname, '../apps/web');
const { renderToStaticMarkup } = createRequire(join(WEB, 'package.json'))('react-dom/server') as typeof import('react-dom/server');
const fill = (s: string, email: string) => s.replace('{email}', email);
const KEYS = [
  'Continue as {email}?',
  'You now sign in with {email}. Another account here already uses that address, so the email on this account stays the same.',
];

describe('addresses inside translated auth text (F1)', () => {
  it('wraps the address in <bdi dir="ltr">, keeping the text around it', () => {
    expect(renderToStaticMarkup(withEmail(fill('Continue as {email}?', EMAIL_SLOT), 'qa@example.com'))).toBe('Continue as <bdi dir="ltr">qa@example.com</bdi>?');
    // The address is text, never markup.
    expect(renderToStaticMarkup(withEmail(`${EMAIL_SLOT}!`, '<b>@x.test'))).toBe('<bdi dir="ltr">&lt;b&gt;@x.test</bdi>!');
  });

  it('works with every locale’s translation, Arabic included', () => {
    for (const code of ['ar', 'de', 'es', 'fr', 'it', 'ja', 'pt-BR', 'zh-CN']) {
      const catalog = JSON.parse(readFileSync(join(WEB, `src/i18n/locales/${code}.json`), 'utf8')) as Record<string, string>;
      for (const key of KEYS) {
        const text = catalog[key];
        expect({ code, key, slots: text?.split('{email}').length }).toEqual({ code, key, slots: 2 });
        const [before, after] = text!.split('{email}');
        expect(renderToStaticMarkup(withEmail(fill(text!, EMAIL_SLOT), 'qa@example.com'))).toBe(
          `${escape(before!)}<bdi dir="ltr">qa@example.com</bdi>${escape(after!)}`,
        );
      }
    }
  });
});

function escape(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#x27;');
}
