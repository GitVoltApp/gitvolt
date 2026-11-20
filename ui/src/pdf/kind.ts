// Whether a file shows as a PDF's pages. No React here: hexContents.ts (in the startup chunk) asks.
import type { DiffContentsPayload } from '../api/gen/DiffContentsPayload';

/** A PDF the viewer shows as pages: the core's `pdf` flag, loaded, and every side there read
 * `raw` (a side that isn't a PDF, such as a Git LFS pointer, isn't: the file shows as text or hex
 * instead). */
export const isPdf = (c: DiffContentsPayload): boolean =>
  !!c.pdf && !c.tooLarge && !!(c.old || c.new) && [c.old, c.new].every((s) => !s || !!s.raw);
