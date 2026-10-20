import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { HostCombobox } from './HostCombobox';

const suggestions = [{ host: 'gitlab.example.com', source: 'origin' }, { host: 'code.example.org', source: 'upstream' }];

describe('HostCombobox', () => {
  it("keeps its list through the window's focus bounce, so a press picks; a real blur closes it", () => {
    const onChange = vi.fn();
    render(<HostCombobox value="" suggestions={suggestions} onChange={onChange} />);
    const input = screen.getByLabelText('Host');
    fireEvent.focus(input);
    const option = screen.getByRole('option', { name: /code\.example\.org/ });
    const has = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    fireEvent.blur(input);
    has.mockRestore();
    expect(option.isConnected).toBe(true);
    fireEvent.mouseDown(option);
    expect(onChange).toHaveBeenCalledWith('code.example.org');
    fireEvent.focus(input);
    fireEvent.blur(input);
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});
