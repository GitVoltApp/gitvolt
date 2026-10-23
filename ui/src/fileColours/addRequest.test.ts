import { afterEach, describe, expect, it, vi } from 'vitest';
import { canOpenFileColours, registerFileColoursOpener, requestAddPattern, useAddPatternRequest } from './addRequest';

afterEach(() => useAddPatternRequest.getState().clear());

describe('add request', () => {
  it('requestAddPattern stores the request and calls the registered opener', () => {
    const open = vi.fn();
    const off = registerFileColoursOpener(open);
    const req = { commonDir: '/r/.git', pattern: '*.snap', via: 'Colour *.snap files…' };
    requestAddPattern(req);
    expect(open).toHaveBeenCalledWith('/r/.git');
    expect(useAddPatternRequest.getState().req).toEqual(req);
    off();
  });
  it('canOpenFileColours is false until an opener registers, and false again after it unregisters', () => {
    expect(canOpenFileColours()).toBe(false);
    const off = registerFileColoursOpener(() => {});
    expect(canOpenFileColours()).toBe(true);
    off();
    expect(canOpenFileColours()).toBe(false);
  });
});
