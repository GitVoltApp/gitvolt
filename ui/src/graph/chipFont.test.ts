// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { useUiFont } from '../theme/fontSettings';
import { chipFont } from './chipFit';

describe('chipFont', () => {
  it('is read once, and again after a UI font change (Settings › Appearance › Fonts)', () => {
    document.body.style.font = '13px Alpha';
    const first = chipFont();
    expect(first).toContain('Alpha');
    document.body.style.font = '13px Beta';
    expect(chipFont()).toBe(first);
    useUiFont.setState({ family: 'Beta' });
    expect(chipFont()).toContain('Beta');
    useUiFont.setState({ family: '' });
    document.body.style.font = '';
  });
});
