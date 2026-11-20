import { describe, expect, it, vi } from 'vitest';
import { base64Bytes } from './sources';

describe('base64Bytes', () => {
  it('reads base64 natively where the engine can, else through atob, to the same bytes', () => {
    const b64 = btoa('%PDF-1.7\n\u00e2\u00e3');
    const expected = Uint8Array.from('%PDF-1.7\n\u00e2\u00e3', (c) => c.charCodeAt(0));
    const native = vi.fn((_s: string) => expected);
    vi.stubGlobal('Uint8Array', Object.assign(class extends Uint8Array {}, { fromBase64: native }));
    try {
      expect(base64Bytes(b64)).toBe(expected);
      expect(native).toHaveBeenCalledWith(b64);
    } finally {
      vi.unstubAllGlobals();
    }
    vi.stubGlobal('Uint8Array', Object.assign(class extends Uint8Array {}, { fromBase64: undefined }));
    try {
      expect(Array.from(base64Bytes(b64))).toEqual(Array.from(expected));
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
