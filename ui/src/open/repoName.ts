/** Destination folder name for a clone URL. */
export function repoNameFromUrl(url: string): string {
  const s = url.trim().replace(/[/\\]+$/, '').replace(/\.git$/i, '');
  const last = s.split(/[/:\\]/).pop() ?? '';
  return last.replace(/[^\w.-]/g, '');
}
