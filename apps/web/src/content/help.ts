/** Help-centre articles (PL-050). Plain paragraphs; `**bold**` and keyboard keys in `[Ctrl]` form are styled. */
export interface HelpArticle { slug: string; title: string; category: string; summary: string; body: string[] }

export const HELP_CATEGORIES = ['Getting started', 'Editing', 'Video', 'Sharing & teams', 'Exporting', 'Account', 'Safety & privacy'] as const;

export const HELP: HelpArticle[] = [
  { slug: 'create-first-design', category: 'Getting started', title: 'Create your first design', summary: 'Start from a size, a template or a blank page.',
    body: ['On Home, choose **Create a design**. Pick a size preset (Instagram post, presentation, A4…) or type a custom size in px, mm, cm or inches.', 'Or open **Templates**, search, and choose **Use template**. Everything in a template can be edited.', 'Designs save automatically while you work, including offline. Rename a design by clicking its title in the editor.'] },
  { slug: 'keyboard-shortcuts', category: 'Getting started', title: 'Keyboard shortcuts', summary: 'The shortcuts that save the most time.',
    body: ['Add text [T], rectangle [R], circle [C], line [L]. Duplicate [Ctrl]+[D], group [Ctrl]+[G], ungroup [Ctrl]+[Shift]+[G]. Add a page [Ctrl]+[Enter].', 'Undo [Ctrl]+[Z], redo [Ctrl]+[Shift]+[Z]. Nudge with arrow keys; hold [Shift] to nudge by 10.', 'Zoom with [Ctrl]+scroll or [Ctrl]+[+]/[-]; fit the page with [Ctrl]+[0]. Play video [Space], split at the playhead [S]. Flip with [Alt]+[Shift]+[H] / [V].', 'On a Mac, use [Cmd] instead of [Ctrl].'] },
  { slug: 'text-and-fonts', category: 'Editing', title: 'Work with text and fonts', summary: 'Styles, effects, lists and spacing.',
    body: ['Double-click a text box to edit. Select words to style only those words.', 'The toolbar sets font, size, colour, bold/italic/underline, alignment, lists, letter and line spacing, and effects such as shadow, outline and highlight.', 'Brand fonts from your brand kit appear at the top of the font list.'] },
  { slug: 'images', category: 'Editing', title: 'Crop, filter and adjust images', summary: 'Upload, crop, flip and colour-correct photos.',
    body: ['Drag images onto the page, paste them, or use **Uploads**. PNG, JPEG, WebP, GIF and SVG are supported.', 'Double-click an image to crop; drag inside to reposition. Use **Edit image** for filters and adjustments (brightness, contrast, saturation, temperature, blur…).'] },
  { slug: 'video-editing', category: 'Video', title: 'Edit video and audio', summary: 'Trim, split, soundtrack and captions.',
    body: ['Upload MP4, MOV or WebM video and MP3, M4A, WAV or OGG audio. Open the **timeline** under the page strip.', 'Select a clip to trim it, change speed and volume, or loop it. Press [S] to split at the playhead.', 'Add a soundtrack from **Video → Soundtrack**.', 'Captions: type them in **Video → Captions** and download them as SRT or VTT.'] },
  { slug: 'share-and-collaborate', category: 'Sharing & teams', title: 'Share and edit together', summary: 'Invite people, set roles, comment and see live cursors.',
    body: ['Click **Share** and add people by email as viewers, commenters or editors, or turn on a link that anyone with it can open.', 'Everyone editing sees live cursors and selections. Leave comments on the page or pinned to an element and @mention people to notify them.', 'Changing someone’s access takes effect immediately, even if they have the design open.'] },
  { slug: 'teams', category: 'Sharing & teams', title: 'Create a team', summary: 'Share designs, brand kits and folders with a group.',
    body: ['In the sidebar choose **Create a team**, then **Members → Invite**. Roles: owners and admins manage members; designers create and edit; members view and comment.'] },
  { slug: 'export-formats', category: 'Exporting', title: 'Choose an export format', summary: 'PNG, JPG, PDF, SVG, MP4, GIF and more.',
    body: ['**PNG** for sharp graphics and transparency; **JPG** for photos at a smaller size; **PDF** for documents and print (PDF Print adds bleed and crop marks); **SVG** for vector graphics with real text.', '**MP4** exports animations and video (720p, 1080p or 4K); **GIF** for short loops; **M4A** for the soundtrack only.', 'Long exports run in the background — you can keep editing and download when ready.'] },
  { slug: 'two-step', category: 'Account', title: 'Secure your account', summary: 'Two-step verification, passkeys and sessions.',
    body: ['Go to Settings → Login & security. Turn on **two-step verification** with an authenticator app and save the recovery codes somewhere safe.', 'Add a **passkey** to sign in with your fingerprint, face or device PIN.', 'The session list shows where you are signed in; log out of any device remotely.'] },
  { slug: 'report-content', category: 'Safety & privacy', title: 'Report harmful or infringing content', summary: 'How reports work and what happens next.',
    body: ['Use **Report** on a public design. Choose the reason; copyright and trademark reports include a legal notice form.', 'Reports are reviewed by severity. Intimate images shared without consent and child-safety reports hide the public link immediately while we review.', 'We email you the outcome. The person affected receives a statement of reasons and can appeal.'] },
  { slug: 'export-delete-data', category: 'Safety & privacy', title: 'Download or delete your data', summary: 'Your data, your call.',
    body: ['Settings → Privacy & data → **Request export** emails you a link to a zip with your profile, designs, comments and uploads.', '**Delete my account** removes your designs, uploads and comments permanently. Designs you made in a team stay with the team.'] },
];
