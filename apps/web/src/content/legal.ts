/**
 * Legal documents shown at /legal/:slug. These are starting templates written for this product.
 * They must be reviewed by counsel for each jurisdiction before a public launch (operator details,
 * governing law and contact addresses are configured per deployment).
 */
import { BRAND } from '@oc/config';

export interface LegalDoc { slug: string; title: string; updated: string; sections: Array<{ h: string; p: string[] }> }

const N = BRAND.name;
const UPDATED = '2026-10-01';

export const LEGAL: LegalDoc[] = [
  {
    slug: 'terms', title: 'Terms of Use', updated: UPDATED,
    sections: [
      { h: 'Who we are', p: [`These terms are an agreement between you and the operator of this ${N} service (“we”). By creating an account or using ${N} you accept them. If you use ${N} for an organisation, you accept them for that organisation.`] },
      { h: 'Your account', p: ['You must be at least 13 years old, or older where local law requires. Keep your sign-in methods secure; you are responsible for activity on your account. Tell us promptly about unauthorised use.'] },
      { h: 'Your content', p: [`You keep ownership of everything you create and upload. You give us a limited licence to host, copy, process and display your content only to run ${N} for you and the people you share with — for example to render thumbnails, sync edits and produce exports.`, 'You promise you have the rights to what you upload and that it does not break the law or our Acceptable Use Policy.'] },
      { h: 'Content we provide', p: [`Templates, graphics, fonts and other media in the ${N} library are licensed to you for use inside your designs. You may not resell them on their own, as templates, or as part of a competing library.`] },
      { h: 'Acceptable use', p: ['You must follow the Acceptable Use Policy. We may remove content or suspend accounts that break it, and we will tell you why and how to appeal unless the law or safety prevents it.'] },
      { h: 'Availability and changes', p: [`We work to keep ${N} available but cannot promise it will never be interrupted. We may change features.`] },
      { h: 'Ending the agreement', p: ['You can delete your account at any time in Settings. We may end the agreement for serious or repeated breaches. After deletion your content is removed as described in the Privacy Policy.'] },
      { h: 'Liability', p: ['Nothing in these terms limits liability that cannot be limited by law. Otherwise, to the extent permitted, we are not liable for indirect losses, and our total liability is limited to the amount you paid us in the 12 months before the claim.'] },
      { h: 'Contact', p: ['Questions about these terms: use Help → Contact support.'] },
    ],
  },
  {
    slug: 'privacy', title: 'Privacy Policy', updated: UPDATED,
    sections: [
      { h: 'Summary', p: [`${N} collects what it needs to run the service, does not sell personal data, and does not use your designs to train AI models. You can download or delete your data at any time from Settings → Privacy & data.`] },
      { h: 'What we collect', p: ['Account data: name, email, password hash, birth year, sign-in methods and security settings.', 'Content: designs, uploads, comments and the version history of your designs.', 'Usage data: device and browser type, IP address, sessions and, only if you allow analytics, product events such as which features you use.'] },
      { h: 'Why we use it', p: ['To provide and secure the service (contract); to send account and security emails (contract, legitimate interest); to improve the product with analytics (consent); to prevent abuse and meet legal obligations (legal obligation, legitimate interest).'] },
      { h: 'Who we share it with', p: ['People you share designs with see your name and the content you share. Processors that host our infrastructure, and deliver email process data on our instructions under contract (see Subprocessors). We disclose data to authorities only when legally required.'] },
      { h: 'How long we keep it', p: ['Account data: while your account exists. Deleted designs: 30 days in the trash, then removed. After account deletion, data is removed from live systems immediately and from backups within 35 days. Security logs: up to 1 year. Analytics events: up to 13 months.'] },
      { h: 'Your rights', p: ['You can access, export, correct and delete your data, object to processing, withdraw consent at any time and complain to your data protection authority. Use Settings → Privacy & data or contact support.'] },
      { h: 'Children', p: [`${N} is not for children under 13. If we learn an account belongs to a child under 13, we delete it.`] },
      { h: 'Security', p: ['Data is encrypted in transit and at rest. Passwords are hashed with scrypt. Two-step verification and passkeys are available to every user.'] },
    ],
  },
  {
    slug: 'cookies', title: 'Cookie Policy', updated: UPDATED,
    sections: [
      { h: 'Essential cookies', p: ['We use a session cookie to keep you signed in and a short-lived cookie to protect sign-in with external providers. These are required for the service and need no consent.'] },
      { h: 'Analytics (optional)', p: ['If you allow analytics, we store a random identifier in your browser and record product events on our own servers. We use no third-party advertising or tracking cookies. You can change your choice at any time with “Cookie settings” at the bottom of the help centre or in Settings → Privacy & data.'] },
      { h: 'Local storage', p: ['The editor keeps an offline copy of designs you open in your browser’s storage so you can keep working without a connection. Signing out does not delete it; clear your browser’s site data to remove it.'] },
    ],
  },
  {
    slug: 'acceptable-use', title: 'Acceptable Use Policy', updated: UPDATED,
    sections: [
      { h: 'Never allowed', p: ['Child sexual abuse material or any sexualisation of minors. Intimate images shared without consent. Content that threatens, incites or glorifies violence or terrorism. Hate speech targeting protected characteristics. Harassment, doxxing or sharing someone’s private information.'] },
      { h: 'Deception and abuse', p: ['Phishing, scams, impersonating people or organisations, fake documents presented as genuine (IDs, certificates, receipts, reviews), malware, and spam.'] },
      { h: 'Intellectual property', p: ['Do not upload content you have no right to use. We respond to copyright and trademark notices and end the accounts of repeat infringers.'] },
      { h: 'Platform integrity', p: ['Do not probe, overload or bypass our security or usage limits, scrape the service, or resell access.'] },
      { h: 'Enforcement', p: ['We may remove content, restrict features or suspend accounts. We send a statement of reasons and you can appeal by replying to it. We report child sexual abuse material to the relevant authorities.'] },
    ],
  },
  {
    slug: 'dmca', title: 'Copyright & IP Notices', updated: UPDATED,
    sections: [
      { h: 'Reporting infringement', p: ['Use the “Report” button on the content and choose “Copyright infringement” or “Trademark infringement”. The form collects what the law requires: your contact details, the original work, where the infringing material is, a good-faith statement, an accuracy statement under penalty of perjury, and your signature.'] },
      { h: 'Counter-notices', p: ['If your content was removed and you believe that was a mistake or that you have the rights, reply to the removal email with a counter-notice. We forward it to the claimant and may restore the content after 10–14 business days unless they take legal action.'] },
      { h: 'Repeat infringers', p: ['Accounts with repeated valid notices are terminated.'] },
    ],
  },
  {
    slug: 'subprocessors', title: 'Subprocessors', updated: UPDATED,
    sections: [
      { h: 'Configured per deployment', p: ['The operator of this deployment lists the providers it uses for hosting, object storage and email delivery here, with the country of processing and the safeguard used for transfers.'] },
    ],
  },
];
