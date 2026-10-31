/** RFC 4180 CSV (and tab-separated, as pasted from spreadsheets) → header + rows. */
export function parseTable(text: string): { headers: string[]; rows: Array<Record<string, string>> } {
  const src = text.replace(/^﻿/, '');
  const firstLine = src.split(/\r?\n/, 1)[0] ?? '';
  const delim = (firstLine.match(/\t/g)?.length ?? 0) > (firstLine.match(/,/g)?.length ?? 0) ? '\t' : firstLine.includes(';') && !firstLine.includes(',') ? ';' : ',';
  const records: string[][] = [];
  let field = '';
  let rec: string[] = [];
  let quoted = false;
  for (let i = 0; i < src.length; i++) {
    const c = src[i]!;
    if (quoted) {
      if (c === '"' && src[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"' && field === '') quoted = true;
    else if (c === delim) { rec.push(field); field = ''; }
    else if (c === '\n' || c === '\r') {
      if (c === '\r' && src[i + 1] === '\n') i++;
      rec.push(field);
      records.push(rec);
      rec = [];
      field = '';
    } else field += c;
  }
  if (field || rec.length) { rec.push(field); records.push(rec); }
  const nonEmpty = records.filter((r) => r.some((v) => v.trim()));
  const headers = (nonEmpty[0] ?? []).map((h, i) => h.trim() || `Column ${i + 1}`);
  const rows = nonEmpty.slice(1).map((r) => Object.fromEntries(headers.map((h, i) => [h, (r[i] ?? '').trim()])));
  return { headers, rows };
}
