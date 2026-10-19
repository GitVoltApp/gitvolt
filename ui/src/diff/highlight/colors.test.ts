import { describe, expect, it } from 'vitest';
import { monacoColors } from './colors';

describe('monacoColors', () => {
  it("keeps Shiki's numbering when Monaco would number the colours the same", () => {
    expect(monacoColors(['', '#D4D4D4', '#1E1E1E', '#569cd6'])).toEqual({ colors: ['#D4D4D4', '#1E1E1E', '#569CD6'], remap: null });
  });

  it('maps colours Monaco merges (alpha, short forms) or can\'t read onto its own ids', () => {
    const { colors, remap } = monacoColors(['', '#D4D4D4', '#1E1E1E', '#D4D4D480', '#fff', 'red', '#FFFFFF']);
    expect(colors).toEqual(['#D4D4D4', '#1E1E1E', '#FFFFFF']);
    expect([...remap!.slice(1, 7)]).toEqual([1, 2, 1, 3, 1, 3]);
  });
});
