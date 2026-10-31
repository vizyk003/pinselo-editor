/**
 * The template catalogue. Each file in this folder registers templates with tpl(); buildTemplates()
 * downloads the pinned photos they reference and builds the design documents.
 */
import { buildRegistered, LIB_ASSETS, type TemplateDef } from './builder';
import { preparePhoto, referencedPhotoIds } from './photos';
import './classic';

/** Canonical category per size preset (the Templates page groups by these). */
export const CATEGORY: Record<string, string> = {
  'ig-post': 'Instagram Posts', 'ig-portrait': 'Instagram Posts', 'ig-story': 'Instagram Stories', 'fb-post': 'Facebook Posts', 'fb-cover': 'Facebook Covers',
  'x-post': 'X Posts', 'linkedin-post': 'LinkedIn Posts', 'linkedin-banner': 'Social Banners', 'pinterest-pin': 'Pinterest Pins', 'yt-thumb': 'YouTube Thumbnails',
  'yt-banner': 'YouTube Banners', presentation: 'Presentations', 'presentation-43': 'Presentations', a4: 'Documents', letter: 'Documents', resume: 'Resumes',
  poster: 'Posters', flyer: 'Flyers', 'business-card': 'Business Cards', invitation: 'Invitations', logo: 'Logos', 'email-header': 'Email Headers',
  infographic: 'Infographics', whiteboard: 'Whiteboards', website: 'Websites', 'link-in-bio': 'Websites', email: 'Emails', doc: 'Docs',
};

export async function buildTemplates(filter?: (p: { id: string; category: string; preset: string }) => boolean): Promise<TemplateDef[]> {
  for (const id of referencedPhotoIds()) await preparePhoto(id);
  return buildRegistered(filter);
}

export { LIB_ASSETS };
export type { TemplateDef };
import './social-a';
import './social-b';
import './video-decks';
import './print';
import './marketing-web';
import './linkinbio';
