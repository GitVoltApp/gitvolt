// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAddPatternRequest } from '../addRequest';
import type { FileScope } from '../store';
import { AddStrip } from './AddStrip';

vi.mock('../../ui/Select', () => ({
  Select: ({ value, options, onChange, ...r }: { value: string; options: [string, string][]; onChange(v: string): void; 'aria-label'?: string }) => (
    <select aria-label={r['aria-label']} value={value} onChange={(e) => onChange(e.target.value)}>{options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
  ),
}));

const sc = (id: string, patterns: string[]): FileScope => ({ id, name: id, tint: 'green', patterns, startCollapsed: false });
const ask = (commonDir = '/r/.git', pattern = '*.snap') => act(() => useAddPatternRequest.setState({ req: { commonDir, pattern, via: 'Colour *.snap files…' } }));
const names = (f: ReturnType<typeof vi.fn>) => f.mock.calls[0][0].map((s: FileScope) => `${s.name}:${s.patterns.join(',')}`);

beforeEach(() => act(() => useAddPatternRequest.setState({ req: null })));
afterEach(cleanup);

describe('AddStrip', () => {
  it('shows the pending pattern and adds it to the chosen scope', () => {
    const onChange = vi.fn();
    render(<AddStrip commonDir="/r/.git" scopes={[sc('A', ['a']), sc('B', ['b'])]} repoPaths={[]} onChange={onChange} />);
    ask();
    expect((screen.getByLabelText('Pattern') as HTMLInputElement).value).toBe('*.snap');
    expect(screen.getByText('From the file list: Colour *.snap files…')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Scope'), { target: { value: 'B' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    expect(names(onChange)).toEqual(['A:a', 'B:b,*.snap']);
    expect(useAddPatternRequest.getState().req).toBeNull();
  });

  it('explains a higher scope that takes the files, and Move puts the scope above it', () => {
    const onChange = vi.fn();
    render(<AddStrip commonDir="/r/.git" scopes={[sc('A', ['**/tests/**']), sc('B', ['*.rs'])]} repoPaths={['a/tests/x.snap', 'y.snap', 'z.rs']} onChange={onChange} />);
    ask();
    fireEvent.change(screen.getByLabelText('Scope'), { target: { value: 'B' } });
    expect(screen.getByText("A is higher in the list and also matches these files, so they'd stay A.")).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /Move B above A/ }));
    expect(names(onChange)).toEqual(['B:*.rs,*.snap', 'A:**/tests/**']);
  });

  it('New scope makes a scope holding the pattern', () => {
    const onChange = vi.fn();
    const onCreated = vi.fn();
    render(<AddStrip commonDir="/r/.git" scopes={[]} repoPaths={null} onChange={onChange} onCreated={onCreated} />);
    ask();
    expect(screen.queryByRole('button', { name: 'Add' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'New scope' }));
    expect(onChange.mock.calls[0][0]).toHaveLength(1);
    expect(onChange.mock.calls[0][0][0].patterns).toEqual(['*.snap']);
    expect(onCreated).toHaveBeenCalledWith(onChange.mock.calls[0][0][0].id);
  });

  it('Esc and Cancel clear the request', () => {
    render(<AddStrip commonDir="/r/.git" scopes={[sc('A', ['a'])]} repoPaths={[]} onChange={vi.fn()} />);
    ask();
    fireEvent.keyDown(screen.getByLabelText('Pattern'), { key: 'Escape' });
    expect(useAddPatternRequest.getState().req).toBeNull();
    ask();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(useAddPatternRequest.getState().req).toBeNull();
  });

  it("a request for another repository isn't shown", () => {
    render(<AddStrip commonDir="/r/.git" scopes={[sc('A', ['a'])]} repoPaths={[]} onChange={vi.fn()} />);
    ask('/other/.git');
    expect(screen.queryByLabelText('Pattern')).toBeNull();
  });
});
