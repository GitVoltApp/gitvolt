import { describe, expect, it } from 'vitest';
import { buttonSets, parseButtonLayout } from './buttonLayout';

describe("GNOME's button-layout", () => {
  it.each([
    // Stock GNOME: the app menu on the left (☰ is always there), only close on the right.
    ['appmenu:close', [], ['close']],
    ['appmenu:minimize,maximize,close', [], ['minimize', 'maximize', 'close']],
    // macOS-style, on the left.
    ['close,minimize,maximize:appmenu', ['close', 'minimize', 'maximize'], []],
    ['close,maximize,minimize:', ['close', 'maximize', 'minimize'], []],
    // Both sides.
    ['close:minimize,maximize', ['close'], ['minimize', 'maximize']],
    // No colon: GTK puts everything on the left.
    ['close', ['close'], []],
    // Nothing at all.
    ['', [], []],
    [':', [], []],
    ['appmenu:', [], []],
    // Empty entries and spaces are separators only.
    [':minimize,,maximize, close', [], ['minimize', 'maximize', 'close']],
    // Unknown names, and GNOME's icon and menu, draw nothing.
    ['icon,menu:help,close', [], ['close']],
  ])('%j', (layout, start, end) => {
    expect(parseButtonLayout(layout)).toEqual({ start, end });
  });

  it('keeps a spacer between buttons, once, and none at an end', () => {
    expect(parseButtonLayout(':minimize,spacer,maximize,close')).toEqual({ start: [], end: ['minimize', 'spacer', 'maximize', 'close'] });
    expect(parseButtonLayout(':minimize,spacer,spacer,close')).toEqual({ start: [], end: ['minimize', 'spacer', 'close'] });
    expect(parseButtonLayout('spacer,close,spacer:spacer')).toEqual({ start: ['close'], end: [] });
  });

  it('draws a button named twice once, where it first appears', () => {
    expect(parseButtonLayout('close:minimize,close')).toEqual({ start: ['close'], end: ['minimize'] });
  });

  it('only Linux follows the layout; Windows has its three on the right, macOS none of its own', () => {
    expect(buttonSets('linux', 'close:')).toEqual({ start: ['close'], end: [] });
    expect(buttonSets('windows', 'close:')).toEqual({ start: [], end: ['minimize', 'maximize', 'close'] });
    expect(buttonSets('macos', 'appmenu:close')).toEqual({ start: [], end: [] });
  });
});
