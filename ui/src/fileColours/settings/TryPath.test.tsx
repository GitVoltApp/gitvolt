// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { FileScope } from '../store';
import { TryPath } from './TryPath';

const scopes: FileScope[] = [
  { id: '1', name: 'Tests', tint: 'green', patterns: ['docs/**', '*.test.*'], startCollapsed: false },
];

afterEach(cleanup);

describe('TryPath', () => {
  it('names the scope and pattern a path lands in, or none', () => {
    render(<TryPath scopes={scopes} />);
    const input = screen.getByLabelText('Try a path');
    fireEvent.change(input, { target: { value: 'src/a.test.ts' } });
    expect(screen.getByText('→ Tests (pattern 2, *.test.*)')).toBeTruthy();
    fireEvent.change(input, { target: { value: 'src/a.ts' } });
    expect(screen.getByText('→ No scope')).toBeTruthy();
  });
});
