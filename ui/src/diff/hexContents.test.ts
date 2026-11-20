import { describe, expect, it, vi } from 'vitest';
import type { BlobPayload } from '../api/gen/BlobPayload';
import type { DiffContentsPayload } from '../api/gen/DiffContentsPayload';

const api = vi.hoisted(() => ({ diffContents: vi.fn(), hexDump: vi.fn(), blobBytes: vi.fn() }));
vi.mock('../api/client', async (actual) => ({ ...(await actual<typeof import('../api/client')>()), api }));
const { bytesOf, hexOf, hexSize, loadContents, PREFETCH_RAW_MAX } = await import('./hexContents');
const { DroppedError } = await import('../data/loader');

const side = (binary: boolean): BlobPayload => ({ size: 4, binary, encoding: '', eol: 'none', text: binary ? null : 'x', hash: null });
const payload = (extra: Partial<DiffContentsPayload>): DiffContentsPayload => ({ old: side(true), new: side(true), tooLarge: false, eolOnly: false, image: false, ...extra });
const req = { path: 'a.bin', old: { kind: 'absent' as const }, new: { kind: 'absent' as const }, force: false };
const hex = { old: { size: 4, shown: 4, dump: 'ab' }, new: null, cap: 262144 };

describe('loadContents', () => {
  it('a prefetch reads raw sides only up to PREFETCH_RAW_MAX in all; an open, or a prefetch an open joined, reads them whatever their size', async () => {
    const raw = (size: number) => ({ ...side(true), size, raw: true });
    api.blobBytes.mockReset().mockResolvedValue(new Uint8Array(1));
    api.diffContents.mockResolvedValueOnce(payload({ image: true, old: raw(PREFETCH_RAW_MAX / 2), new: raw(PREFETCH_RAW_MAX / 2 + 1) }));
    await expect(loadContents(1, req, () => true)).rejects.toBeInstanceOf(DroppedError);
    expect(api.blobBytes).not.toHaveBeenCalled();
    api.diffContents.mockResolvedValueOnce(payload({ image: true, old: null, new: raw(PREFETCH_RAW_MAX) }));
    expect(bytesOf((await loadContents(1, req, () => true)).new)).toHaveLength(1);
    api.diffContents.mockResolvedValueOnce(payload({ image: true, old: null, new: raw(16 << 20) }));
    expect(bytesOf((await loadContents(1, req)).new)).toHaveLength(1);
    let prefetching = true;
    api.diffContents.mockImplementationOnce(async () => { prefetching = false; return payload({ image: true, old: null, new: raw(16 << 20) }); });
    expect(bytesOf((await loadContents(1, req, () => prefetching)).new)).toHaveLength(1);
    api.blobBytes.mockReset();
  });


  it('loads a binary\'s hex dumps with its contents; not a text file\'s, an image\'s or a large file\'s', async () => {
    api.hexDump.mockResolvedValue(hex);
    api.diffContents.mockResolvedValueOnce(payload({}));
    const bin = await loadContents(1, req);
    expect(hexOf(bin)).toEqual(hex);
    expect(hexSize(bin)).toBe(4);
    expect(api.hexDump).toHaveBeenCalledWith(1, req);
    for (const p of [payload({ old: side(false), new: side(false) }), payload({ image: true }), payload({ tooLarge: true })]) {
      api.hexDump.mockClear();
      api.diffContents.mockResolvedValueOnce(p);
      const c = await loadContents(1, req);
      expect(hexOf(c)).toBeNull();
      expect(api.hexDump).not.toHaveBeenCalled();
    }
  });

  it('reads the raw sides\' bytes (blobBytes, both at once, by the request\'s path and sides) into the loaded contents', async () => {
    const png = { ...side(true), raw: true };
    const old = { kind: 'object' as const, oid: 'a'.repeat(40) };
    const neu = { kind: 'worktree' as const, worktree: '/w' };
    const pending: ((b: Uint8Array) => void)[] = [];
    api.blobBytes.mockImplementation(() => new Promise((resolve) => pending.push(resolve)));
    api.diffContents.mockResolvedValueOnce(payload({ image: true, old: png, new: png }));
    const loading = loadContents(1, { ...req, path: 'shot.png', old, new: neu });
    await vi.waitFor(() => expect(pending).toHaveLength(2));
    expect(api.blobBytes.mock.calls).toEqual([[1, 'shot.png', old], [1, 'shot.png', neu]]);
    pending[1](new Uint8Array([2, 2]));
    pending[0](new Uint8Array([1]));
    const c = await loading;
    expect([...bytesOf(c.old)!]).toEqual([1]);
    expect([...bytesOf(c.new)!]).toEqual([2, 2]);
    // Not raw: never asked.
    api.blobBytes.mockClear();
    api.diffContents.mockResolvedValueOnce(payload({ image: true, old: null, new: { ...side(false), text: '<svg/>' } }));
    expect(bytesOf((await loadContents(1, req)).new)).toBeNull();
    expect(api.blobBytes).not.toHaveBeenCalled();
  });
});
