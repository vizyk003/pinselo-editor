/**
 * Pulls open-licence (OFL / Apache / UFL) font families (TTF) from the @expo-google-fonts/* npm packages
 * into fixtures/fonts/files/ and writes fixtures/fonts/manifest.json with sha256 pins.
 *
 * Browser and server load the exact same files (ADR-0043), which is what makes
 * client/server text layout parity possible.
 */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '../..');
const OUT = join(ROOT, 'fixtures/fonts/files');

/** [display family, npm slug, category] */
const FAMILIES: Array<[string, string, string]> = [
  ['Inter', 'inter', 'sans'],
  ['Roboto', 'roboto', 'sans'],
  ['Open Sans', 'open-sans', 'sans'],
  ['Montserrat', 'montserrat', 'sans'],
  ['Poppins', 'poppins', 'sans'],
  ['Lato', 'lato', 'sans'],
  ['Raleway', 'raleway', 'sans'],
  ['Nunito', 'nunito', 'sans'],
  ['Rubik', 'rubik', 'sans'],
  ['Space Grotesk', 'space-grotesk', 'sans'],
  ['Josefin Sans', 'josefin-sans', 'sans'],
  ['Work Sans', 'work-sans', 'sans'],
  ['DM Sans', 'dm-sans', 'sans'],
  ['Manrope', 'manrope', 'sans'],
  ['Plus Jakarta Sans', 'plus-jakarta-sans', 'sans'],
  ['Outfit', 'outfit', 'sans'],
  ['Urbanist', 'urbanist', 'sans'],
  ['Sora', 'sora', 'sans'],
  ['Lexend', 'lexend', 'sans'],
  ['Figtree', 'figtree', 'sans'],
  ['Karla', 'karla', 'sans'],
  ['Mulish', 'mulish', 'sans'],
  ['Quicksand', 'quicksand', 'sans'],
  ['Barlow', 'barlow', 'sans'],
  ['Barlow Condensed', 'barlow-condensed', 'sans'],
  ['Fira Sans', 'fira-sans', 'sans'],
  ['IBM Plex Sans', 'ibm-plex-sans', 'sans'],
  ['Source Sans 3', 'source-sans-3', 'sans'],
  ['PT Sans', 'pt-sans', 'sans'],
  ['Noto Sans', 'noto-sans', 'sans'],
  ['Ubuntu', 'ubuntu', 'sans'],
  ['Cabin', 'cabin', 'sans'],
  ['Hind', 'hind', 'sans'],
  ['Heebo', 'heebo', 'sans'],
  ['Assistant', 'assistant', 'sans'],
  ['Exo 2', 'exo-2', 'sans'],
  ['Kanit', 'kanit', 'sans'],
  ['Titillium Web', 'titillium-web', 'sans'],
  ['Varela Round', 'varela-round', 'sans'],
  ['Comfortaa', 'comfortaa', 'sans'],
  ['Red Hat Display', 'red-hat-display', 'sans'],
  ['Public Sans', 'public-sans', 'sans'],
  ['Archivo', 'archivo', 'sans'],
  ['Albert Sans', 'albert-sans', 'sans'],
  ['Onest', 'onest', 'sans'],
  ['Be Vietnam Pro', 'be-vietnam-pro', 'sans'],
  ['Epilogue', 'epilogue', 'sans'],
  ['Syne', 'syne', 'sans'],
  ['Chivo', 'chivo', 'sans'],
  ['Overpass', 'overpass', 'sans'],
  ['Maven Pro', 'maven-pro', 'sans'],
  ['Questrial', 'questrial', 'sans'],
  ['Jost', 'jost', 'sans'],
  ['Prompt', 'prompt', 'sans'],
  ['Encode Sans', 'encode-sans', 'sans'],
  ['M PLUS Rounded 1c', 'm-plus-rounded-1c', 'sans'],
  ['Libre Franklin', 'libre-franklin', 'sans'],
  ['Spline Sans', 'spline-sans', 'sans'],
  ['Oswald', 'oswald', 'display'],
  ['Bebas Neue', 'bebas-neue', 'display'],
  ['Anton', 'anton', 'display'],
  ['Archivo Black', 'archivo-black', 'display'],
  ['Abril Fatface', 'abril-fatface', 'display'],
  ['Alfa Slab One', 'alfa-slab-one', 'display'],
  ['Bungee', 'bungee', 'display'],
  ['Righteous', 'righteous', 'display'],
  ['Russo One', 'russo-one', 'display'],
  ['Black Ops One', 'black-ops-one', 'display'],
  ['Bowlby One SC', 'bowlby-one-sc', 'display'],
  ['Lilita One', 'lilita-one', 'display'],
  ['Luckiest Guy', 'luckiest-guy', 'display'],
  ['Fredoka', 'fredoka', 'display'],
  ['Paytone One', 'paytone-one', 'display'],
  ['Passion One', 'passion-one', 'display'],
  ['Staatliches', 'staatliches', 'display'],
  ['Teko', 'teko', 'display'],
  ['Rammetto One', 'rammetto-one', 'display'],
  ['Ultra', 'ultra', 'display'],
  ['Chango', 'chango', 'display'],
  ['Monoton', 'monoton', 'display'],
  ['Bungee Shade', 'bungee-shade', 'display'],
  ['Shrikhand', 'shrikhand', 'display'],
  ['Titan One', 'titan-one', 'display'],
  ['Carter One', 'carter-one', 'display'],
  ['Bangers', 'bangers', 'display'],
  ['Press Start 2P', 'press-start-2p', 'display'],
  ['Orbitron', 'orbitron', 'display'],
  ['Audiowide', 'audiowide', 'display'],
  ['Rubik Mono One', 'rubik-mono-one', 'display'],
  ['Big Shoulders Display', 'big-shoulders-display', 'display'],
  ['Unbounded', 'unbounded', 'display'],
  ['Dela Gothic One', 'dela-gothic-one', 'display'],
  ['Climate Crisis', 'climate-crisis', 'display'],
  ['Bricolage Grotesque', 'bricolage-grotesque', 'display'],
  ['Playfair Display', 'playfair-display', 'serif'],
  ['Lora', 'lora', 'serif'],
  ['Merriweather', 'merriweather', 'serif'],
  ['DM Serif Display', 'dm-serif-display', 'serif'],
  ['Libre Baskerville', 'libre-baskerville', 'serif'],
  ['Cormorant Garamond', 'cormorant-garamond', 'serif'],
  ['EB Garamond', 'eb-garamond', 'serif'],
  ['Crimson Text', 'crimson-text', 'serif'],
  ['PT Serif', 'pt-serif', 'serif'],
  ['Noto Serif', 'noto-serif', 'serif'],
  ['Source Serif 4', 'source-serif-4', 'serif'],
  ['Bitter', 'bitter', 'serif'],
  ['Roboto Slab', 'roboto-slab', 'serif'],
  ['Zilla Slab', 'zilla-slab', 'serif'],
  ['Arvo', 'arvo', 'serif'],
  ['Cinzel', 'cinzel', 'serif'],
  ['Prata', 'prata', 'serif'],
  ['Spectral', 'spectral', 'serif'],
  ['Fraunces', 'fraunces', 'serif'],
  ['Young Serif', 'young-serif', 'serif'],
  ['Gloock', 'gloock', 'serif'],
  ['Bodoni Moda', 'bodoni-moda', 'serif'],
  ['Libre Caslon Text', 'libre-caslon-text', 'serif'],
  ['Cardo', 'cardo', 'serif'],
  ['Old Standard TT', 'old-standard-tt', 'serif'],
  ['Rozha One', 'rozha-one', 'serif'],
  ['Yeseva One', 'yeseva-one', 'serif'],
  ['Instrument Serif', 'instrument-serif', 'serif'],
  ['Marcellus', 'marcellus', 'serif'],
  ['Cormorant', 'cormorant', 'serif'],
  ['DM Serif Text', 'dm-serif-text', 'serif'],
  ['Domine', 'domine', 'serif'],
  ['Vollkorn', 'vollkorn', 'serif'],
  ['Alegreya', 'alegreya', 'serif'],
  ['Literata', 'literata', 'serif'],
  ['Pacifico', 'pacifico', 'script'],
  ['Dancing Script', 'dancing-script', 'script'],
  ['Great Vibes', 'great-vibes', 'script'],
  ['Sacramento', 'sacramento', 'script'],
  ['Allura', 'allura', 'script'],
  ['Alex Brush', 'alex-brush', 'script'],
  ['Parisienne', 'parisienne', 'script'],
  ['Satisfy', 'satisfy', 'script'],
  ['Kaushan Script', 'kaushan-script', 'script'],
  ['Lobster', 'lobster', 'script'],
  ['Yellowtail', 'yellowtail', 'script'],
  ['Cookie', 'cookie', 'script'],
  ['Tangerine', 'tangerine', 'script'],
  ['Pinyon Script', 'pinyon-script', 'script'],
  ['Marck Script', 'marck-script', 'script'],
  ['Courgette', 'courgette', 'script'],
  ['Damion', 'damion', 'script'],
  ['Mr Dafoe', 'mr-dafoe', 'script'],
  ['Monsieur La Doulaise', 'monsieur-la-doulaise', 'script'],
  ['Italianno', 'italianno', 'script'],
  ['Petit Formal Script', 'petit-formal-script', 'script'],
  ['Herr Von Muellerhoff', 'herr-von-muellerhoff', 'script'],
  ['Lobster Two', 'lobster-two', 'script'],
  ['Playball', 'playball', 'script'],
  ['Rochester', 'rochester', 'script'],
  ['Caveat', 'caveat', 'handwriting'],
  ['Indie Flower', 'indie-flower', 'handwriting'],
  ['Shadows Into Light', 'shadows-into-light', 'handwriting'],
  ['Amatic SC', 'amatic-sc', 'handwriting'],
  ['Permanent Marker', 'permanent-marker', 'handwriting'],
  ['Patrick Hand', 'patrick-hand', 'handwriting'],
  ['Gloria Hallelujah', 'gloria-hallelujah', 'handwriting'],
  ['Kalam', 'kalam', 'handwriting'],
  ['Architects Daughter', 'architects-daughter', 'handwriting'],
  ['Rock Salt', 'rock-salt', 'handwriting'],
  ['Covered By Your Grace', 'covered-by-your-grace', 'handwriting'],
  ['Reenie Beanie', 'reenie-beanie', 'handwriting'],
  ['Homemade Apple', 'homemade-apple', 'handwriting'],
  ['Nanum Pen Script', 'nanum-pen-script', 'handwriting'],
  ['Just Another Hand', 'just-another-hand', 'handwriting'],
  ['Coming Soon', 'coming-soon', 'handwriting'],
  ['Handlee', 'handlee', 'handwriting'],
  ['Nothing You Could Do', 'nothing-you-could-do', 'handwriting'],
  ['Schoolbell', 'schoolbell', 'handwriting'],
  ['Gochi Hand', 'gochi-hand', 'handwriting'],
  ['Sue Ellen Francisco', 'sue-ellen-francisco', 'handwriting'],
  ['Caveat Brush', 'caveat-brush', 'handwriting'],
  ['JetBrains Mono', 'jetbrains-mono', 'mono'],
  ['Fira Code', 'fira-code', 'mono'],
  ['Space Mono', 'space-mono', 'mono'],
  ['IBM Plex Mono', 'ibm-plex-mono', 'mono'],
  ['Source Code Pro', 'source-code-pro', 'mono'],
  ['Roboto Mono', 'roboto-mono', 'mono'],
  ['DM Mono', 'dm-mono', 'mono'],
  ['Courier Prime', 'courier-prime', 'mono'],
  ['Inconsolata', 'inconsolata', 'mono'],
  ['Major Mono Display', 'major-mono-display', 'mono'],
];

const WANTED = /^(\d00)([A-Za-z]+?)(_Italic)?$/;
const KEEP_WEIGHTS_UPRIGHT = new Set([300, 400, 500, 600, 700, 800, 900]);
const KEEP_WEIGHTS_ITALIC = new Set([400, 700]);

interface FaceEntry { weight: number; italic: boolean; file: string; sha256: string; bytes: number }
interface FamilyEntry { family: string; slug: string; category: string; license: 'OFL-1.1' | 'Apache-2.0' | 'UFL-1.0'; faces: FaceEntry[] }

mkdirSync(OUT, { recursive: true });
const tmp = mkdtempSync(join(tmpdir(), 'oc-fonts-'));
const manifest: FamilyEntry[] = [];

/** Exact package versions (fonts-pins.json), so a sync reproduces the committed manifest.json; an unpinned family takes
 * the latest version and asks for a pin. */
const PINS = JSON.parse(readFileSync(join(import.meta.dirname, 'fonts-pins.json'), 'utf8')) as Record<string, string>;

for (const [family, slug, category] of FAMILIES) {
  if (!PINS[slug]) console.warn(`${slug}: no pin in tools/scripts/fonts-pins.json, using the latest version (add one)`);
  const pkg = `@expo-google-fonts/${slug}${PINS[slug] ? `@${PINS[slug]}` : ''}`;
  const dest = join(OUT, slug);
  const work = join(tmp, slug);
  mkdirSync(work, { recursive: true });
  process.stdout.write(`${family}… `);
  let tgz: string;
  try {
    tgz = execFileSync('npm', ['pack', pkg, '--silent', '--pack-destination', work], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim().split('\n').pop()!;
  } catch (e) {
    // No npm at all is a broken environment, not a missing package: fail instead of writing an empty font set.
    if ((e as NodeJS.ErrnoException).code === 'ENOENT') throw new Error('fonts:sync needs npm on PATH (it fetches the @expo-google-fonts packages with `npm pack`)');
    console.log('not on npm, skipped');
    continue;
  }
  execFileSync('tar', ['xzf', join(work, tgz), '-C', work]);
  const pkgDir = join(work, 'package');
  mkdirSync(dest, { recursive: true });
  const faces: FaceEntry[] = [];
  for (const dir of readdirSync(pkgDir)) {
    const m = WANTED.exec(dir);
    if (!m) continue;
    const weight = Number(m[1]);
    const italic = Boolean(m[3]);
    if (!(italic ? KEEP_WEIGHTS_ITALIC : KEEP_WEIGHTS_UPRIGHT).has(weight)) continue;
    const ttf = readdirSync(join(pkgDir, dir)).find((f) => f.endsWith('.ttf'));
    if (!ttf) continue;
    cpSync(join(pkgDir, dir, ttf), join(dest, ttf));
    const buf = readFileSync(join(dest, ttf));
    faces.push({
      weight,
      italic,
      file: `${slug}/${ttf}`,
      sha256: createHash('sha256').update(buf).digest('hex'),
      bytes: buf.length,
    });
  }
  let license: FamilyEntry['license'] = 'OFL-1.1';
  if (existsSync(join(pkgDir, 'LICENSE_FONT'))) {
    const text = readFileSync(join(pkgDir, 'LICENSE_FONT'), 'utf8');
    license = /Apache License/i.test(text) ? 'Apache-2.0' : /Ubuntu Font Licen[cs]e/i.test(text) ? 'UFL-1.0' : 'OFL-1.1';
    cpSync(join(pkgDir, 'LICENSE_FONT'), join(dest, license === 'OFL-1.1' ? 'OFL.txt' : 'LICENSE.txt'));
  }
  if (!faces.length) {
    console.log('no usable faces, skipped');
    continue;
  }
  faces.sort((a, b) => a.weight - b.weight || Number(a.italic) - Number(b.italic));
  manifest.push({ family, slug, category, license, faces });
  console.log(`${faces.length} faces`);
}

rmSync(tmp, { recursive: true, force: true });
writeFileSync(join(ROOT, 'fixtures/fonts/manifest.json'), `${JSON.stringify({ version: 1, families: manifest }, null, 2)}\n`);
console.log(`Wrote ${manifest.length} families to fixtures/fonts/manifest.json`);
