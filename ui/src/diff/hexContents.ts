// A binary's hex dumps load with its contents (the services' `contents` loader), so a binary is
// ready to show in the same render as a text file is: the panel's editor stays mounted and keeps
// the previous file until the dumps' diff is on screen. So do the bytes of the sides the core
// marks `raw` (images, PDFs, media). No React or Monaco here: the services module is in the
// startup chunk.
import { api, type ContentsRequest } from '../api/client';
import type { BlobPayload } from '../api/gen/BlobPayload';
import type { BlobSource } from '../api/gen/BlobSource';
import type { DiffContentsPayload } from '../api/gen/DiffContentsPayload';
import type { HexDumpPayload } from '../api/gen/HexDumpPayload';
import { showsMedia } from '../media/kind';
import { isPdf } from '../pdf/kind';

/** Contents, with a binary's hex dumps (`hexDump`) when it has them. */
export type HexContents = DiffContentsPayload & { hex?: HexDumpPayload };

/** A binary loaded for the viewer that isn't a raster image, media or a PDF: it shows as hex.
 * (An SVG with a binary side has no image bytes, so it's one too.) */
export const wantsHex = (c: DiffContentsPayload) => !c.tooLarge && !c.image && !showsMedia(c) && !isPdf(c) && !!(c.old?.binary || c.new?.binary);

export const hexOf = (c: DiffContentsPayload): HexDumpPayload | null => (c as HexContents).hex ?? null;

/** A side the core marked `raw`, with its bytes (`blobBytes`). */
export type RawBlob = BlobPayload & { bytes?: Uint8Array<ArrayBuffer> };

/** A loaded side's raw bytes: `null` for a side that isn't `raw` (text, a binary shown as hex). */
export const bytesOf = (b: BlobPayload | null | undefined): Uint8Array<ArrayBuffer> | null => (b as RawBlob | null | undefined)?.bytes ?? null;

/** `diffContents`, then, for a binary, `hexDump` of the same sides, else the `raw` sides' bytes
 * (`blobBytes`, both at once). */
export async function loadContents(repo: number, req: ContentsRequest): Promise<HexContents> {
  const c = await api.diffContents(repo, req);
  if (wantsHex(c)) return { ...c, hex: await api.hexDump(repo, req) };
  if (!c.old?.raw && !c.new?.raw) return c;
  // Each side is read by the request's path, as `diffContents` reads both.
  const read = async (b: BlobPayload | null, side: BlobSource): Promise<RawBlob | null> => (b?.raw ? { ...b, bytes: await api.blobBytes(repo, req.path, side) } : b);
  const [old, neu] = await Promise.all([read(c.old, req.old), read(c.new, req.new)]);
  return { ...c, old, new: neu };
}

/** The dumps' share of the cache's size (UTF-16). */
export const hexSize = (c: DiffContentsPayload) => {
  const h = hexOf(c);
  return h ? ((h.old?.dump.length ?? 0) + (h.new?.dump.length ?? 0)) * 2 : 0;
};
