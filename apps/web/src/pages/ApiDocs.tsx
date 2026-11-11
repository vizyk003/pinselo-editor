/** Public developer guide (/developers/api): publishing designs through the REST API — read a design's version, export it
 * as JPEGs pinned to that version, hand the public links to a social network. English only, like the help articles. The
 * full reference is the OpenAPI document (/v1/openapi.json) and docs/api.md. */
import { BRAND } from '@oc/config';
import { useEffect, type ReactNode } from 'react';
import { Link } from 'react-router';
import { PublicShell } from './Public';

function Code({ children }: { children: string }) {
  return <pre><code>{children.trim()}</code></pre>;
}

function Table({ head, rows }: { head: string[]; rows: ReactNode[][] }) {
  return (
    <div className="table-wrap">
      <table>
        <thead><tr>{head.map((h) => <th key={h}>{h}</th>)}</tr></thead>
        <tbody>{rows.map((r, i) => <tr key={i}>{r.map((c, j) => <td key={j}>{c}</td>)}</tr>)}</tbody>
      </table>
    </div>
  );
}

export function ApiDocsPage() {
  const origin = typeof window === 'undefined' ? 'https://pinselo.com' : window.location.origin;
  useEffect(() => {
    document.title = `REST API: publishing designs – ${BRAND.name}`;
  }, []);
  return (
    <PublicShell>
      <article className="prose">
        <h1>REST API: publishing designs</h1>
        <p className="faint">For tools that post finished designs on a schedule, such as an Instagram publisher.</p>
        <p>
          A publishing tool makes three calls per post. It reads the design to check its version, exports the pages as JPEGs
          pinned to that version, and passes the file links to the social network. If anyone edited the design after it was
          approved, the export is refused, so nothing unreviewed goes out.
        </p>

        <h2>1. Create a token</h2>
        <p>
          In {BRAND.name}, open <b>Settings → Developers → API access tokens</b>. Give the token a name, pick the workspace
          that holds your designs under <b>Designs</b>, and tick the <code>designs:read</code> and <code>designs:export</code> scopes.
        </p>
        <ul>
          <li>The token is shown once. Store it as a secret in your tool, never in a spreadsheet, a calendar or a chat.</li>
          <li>A token limited to one workspace can only read and export. It can't edit or delete anything, and designs in other workspaces look like they don't exist.</li>
          <li>Revoking the token takes effect on the next request, and its export links stop working too.</li>
        </ul>
        <p>Send the token in a header on every call. Tokens in the URL are never accepted.</p>
        <Code>{`Authorization: Bearer oc_pat_…`}</Code>

        <h2>2. Read the design</h2>
        <p>
          Record the design's <code>version</code> when the post is approved. It goes up with every saved change, including
          autosave. Before posting, read the design again and compare.
        </p>
        <Code>{`curl ${origin}/v1/designs/WgYq99H11jFflr5B \\
  -H "Authorization: Bearer $PINSELO_TOKEN"`}</Code>
        <Code>{`{
  "id": "WgYq99H11jFflr5B",
  "title": "P-002 fee fact card",
  "version": 14,
  "width": 1080,
  "height": 1350,
  "pages": 3,
  "pageList": [
    { "index": 1, "id": "pg_a1", "name": "Cover", "hidden": false },
    { "index": 2, "id": "pg_b2", "name": "Page 2", "hidden": false },
    { "index": 3, "id": "pg_c3", "name": "Page 3", "hidden": false }
  ],
  "updatedAt": "2026-11-11T10:52:03.000Z",
  "editUrl": "${origin}/design/WgYq99H11jFflr5B"
}`}</Code>
        <p>
          The id can also be the design's edit link, URL-encoded, if that's what your calendar stores. An unknown design, or
          one the token can't see, answers <code>404</code>.
        </p>

        <h2>3. Export the pages</h2>
        <Code>{`curl ${origin}/v1/designs/WgYq99H11jFflr5B/exports \\
  -H "Authorization: Bearer $PINSELO_TOKEN" \\
  -H "Content-Type: application/json" \\
  -H "Idempotency-Key: P-002-v14" \\
  -d '{"format":"jpeg","version":14,"width":1080,"pages":[1,2,3],"wait":true}'`}</Code>
        <Table
          head={['Field', 'Meaning']}
          rows={[
            [<code>format</code>, <><code>"jpeg"</code>. Required.</>],
            [<code>version</code>, 'The approved version. If the design has changed since, the export is refused with 409 and nothing is rendered. Optional, but send it.'],
            [<code>width</code>, 'Output width in px, 320 to 1440. Default 1080. The height follows the design.'],
            [<code>quality</code>, 'JPEG quality, 60 to 100. Default 90.'],
            [<code>pages</code>, 'Page numbers in the order you want the files, 1 being the first. At most 10. Default: every visible page.'],
            [<code>wait</code>, <><code>true</code> keeps the request open for up to 25 seconds until the files are ready.</>],
          ]}
        />
        <p>
          The <code>Idempotency-Key</code> header is optional. For 24 hours, sending the same key again returns the same
          export and links without rendering again, so retries are safe. Using your post id plus the version works well.
        </p>
        <Code>{`{
  "id": "exp_7Hq2uY1pQ0aZ3cV8bN5mKw",
  "status": "done",
  "designId": "WgYq99H11jFflr5B",
  "designVersion": 14,
  "warnings": [],
  "files": [
    {
      "page": 1,
      "url": "${origin}/v1/files/exp_7Hq2uY1pQ0aZ3cV8bN5mKw/1.jpg?e=1794763390&sig=…",
      "width": 1080,
      "height": 1350,
      "bytes": 412883,
      "contentType": "image/jpeg"
    }
  ],
  "expiresAt": "2026-11-15T17:23:10.552Z",
  "statusUrl": "${origin}/v1/exports/exp_7Hq2uY1pQ0aZ3cV8bN5mKw"
}`}</Code>

        <h2>4. If it's still rendering</h2>
        <p>
          Long carousels can take more than 25 seconds. The export then answers <code>202</code> with <code>"status": "rendering"</code>.
          Poll <code>statusUrl</code> every few seconds until <code>status</code> is <code>done</code> or <code>failed</code>.
        </p>
        <Code>{`curl ${origin}/v1/exports/exp_7Hq2uY1pQ0aZ3cV8bN5mKw \\
  -H "Authorization: Bearer $PINSELO_TOKEN"`}</Code>
        <p>A failed export answers <code>200</code> with <code>"status": "failed"</code> and an <code>error</code> object holding a <code>code</code> and a <code>message</code>.</p>

        <h2>5. Post the files</h2>
        <p>Pass each <code>files[].url</code> to the social network's API, in order. The links are made for that.</p>
        <ul>
          <li>Anyone can download them for 24 hours (<code>expiresAt</code>) with a plain GET: no login, no cookies, no headers.</li>
          <li>They answer <code>Content-Type: image/jpeg</code> with no redirects, and the path ends in <code>.jpg</code>.</li>
          <li>Each file is a baseline JPEG with the sRGB profile embedded and no transparency (flattened onto white), and is at most 7.5 MB. That meets Instagram's image rules.</li>
          <li>Fonts must load. A page whose font can't be loaded fails the export instead of rendering a substitute font.</li>
          <li><code>warnings</code> flags a design taller than 4:5 or wider than 1.91:1, which Instagram's feed doesn't accept as is.</li>
        </ul>

        <h2>Errors</h2>
        <p>Errors are JSON with an <code>error</code> code and a <code>message</code>. A 409 also carries <code>currentVersion</code>.</p>
        <Table
          head={['Status', 'error', 'What to do']}
          rows={[
            ['401', <><code>unauthenticated</code>, <code>invalid_token</code></>, 'The token is missing, wrong or revoked. Stop and report.'],
            ['403', <code>insufficient_scope</code>, 'The token lacks designs:read or designs:export. Stop and report.'],
            ['404', <code>not_found</code>, 'The design doesn’t exist or isn’t in the token’s workspace. Mark the post failed.'],
            ['409', <code>version_changed</code>, 'The design was edited after approval. Hold the post until it’s approved again.'],
            ['422', <code>invalid_export</code>, 'A bad field, a page out of range, or a page still over 7.5 MB at quality 60. The message says which. Mark the post failed.'],
            ['429', <code>rate_limited</code>, 'Too many requests. Wait the Retry-After seconds, then retry once.'],
            ['500', <code>render_failed</code>, 'Rendering failed, for example because a font couldn’t load. Retry once later.'],
            ['503', <code>unavailable</code>, 'A service is briefly unreachable. Retry once later.'],
          ]}
        />

        <h2>Limits</h2>
        <ul>
          <li>120 requests a minute per token for each endpoint. Every response carries <code>X-RateLimit-Remaining</code>.</li>
          <li>200 exports a day per token. Idempotent replays don't count.</li>
          <li>At most 10 pages per export.</li>
        </ul>

        <h2>Full reference</h2>
        <p>
          The API can also list, create and fill designs, upload assets, export PDF, PowerPoint and video, and send webhooks.
          Every endpoint is described in the <a href="/v1/openapi.json" target="_blank" rel="noreferrer">OpenAPI document</a>.
          Questions? <Link to="/help/contact">Contact us</Link>.
        </p>
      </article>
    </PublicShell>
  );
}
