import { create } from 'zustand';

/** A pattern the file list's menu asks the File colours page to add. `via` is the menu item's
 * label, for the strip's "From the file list: …". */
export interface AddPatternRequest { commonDir: string; pattern: string; via: string }

export const useAddPatternRequest = create<{ req: AddPatternRequest | null; clear(): void }>((set) => ({
  req: null,
  clear: () => set({ req: null }),
}));

let opener: ((commonDir: string) => void) | null = null;

/** The settings page registers how to open File colours; the menu's items exist only while it can. */
export function registerFileColoursOpener(open: (commonDir: string) => void): () => void {
  opener = open;
  return () => { if (opener === open) opener = null; };
}

export const canOpenFileColours = (): boolean => opener !== null;

export function requestAddPattern(req: AddPatternRequest): void {
  useAddPatternRequest.setState({ req });
  opener?.(req.commonDir);
}
