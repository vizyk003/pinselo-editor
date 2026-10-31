/** One-shot hint for the Download dialog: open with only the current page picked ("Download this page", g97). */
let onePage = false;

export function requestOnePageDownload() {
  onePage = true;
}

/** Read and clear the hint. */
export function takeOnePageDownload(): boolean {
  const v = onePage;
  onePage = false;
  return v;
}
