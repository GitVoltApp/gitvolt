import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { LivePreview } from './LivePreview';

describe('the live preview', () => {
  it('draws graph rows, a file and diff lines in the theme tokens, hidden from assistive tech', () => {
    const { container } = render(<LivePreview />);
    const root = container.firstElementChild!;
    expect(root).toHaveAttribute('aria-hidden');
    expect(root.querySelectorAll('.theme-preview-row')).toHaveLength(2);
    expect(root.querySelector('.theme-preview-row.selected')).not.toBeNull();
    expect(root.querySelector('.theme-preview-file')).toHaveTextContent('Meconomy.rs');
    expect(root.querySelectorAll('.theme-preview-line')).toHaveLength(3);
    expect(root.innerHTML).toContain('var(--diff-removed-line)');
    expect(root.innerHTML).toContain('var(--graph-1)');
  });
});
