import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { useUiFont } from '../theme/fontSettings';
import { CsvTable } from './CsvTable';
import { parseTable } from './csv';

// A canvas whose letters are twice as wide in the "Wide" font (digits, the `ch` unit, are not):
// set before the table first measures, which keeps the context.
HTMLCanvasElement.prototype.getContext = function getContext() {
  const ctx = { font: '', measureText: (t: string) => ({ width: [...t].reduce((w, c) => w + (/\d/.test(c) || !ctx.font.includes('Wide') ? 10 : 20), 0) }) };
  return ctx;
} as never;

afterEach(() => {
  cleanup();
  document.documentElement.style.removeProperty('--font-ui');
  useUiFont.setState({ family: '' });
});

describe('CsvTable and the UI font', () => {
  it('measures its columns again in a new UI font (Settings › Appearance › Fonts)', () => {
    const text = 'id,name\n1,abcd\n';
    render(<CsvTable text={text} table={parseTable(text, ',')} fontPx={13} />);
    const table = screen.getByRole('table');
    const before = table.style.getPropertyValue('--csv-cols');
    expect(before).toContain('calc(4ch + 17px)');
    act(() => {
      document.documentElement.style.setProperty('--font-ui', 'Wide, sans-serif');
      useUiFont.setState({ family: 'Wide, sans-serif' });
    });
    expect(table.style.getPropertyValue('--csv-cols')).toContain('calc(8ch + 17px)');
  });
});
