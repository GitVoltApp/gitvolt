import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useAppState, EMPTY_PROFILE } from '../app/state';
import { ContextMenu } from '../menu/ContextMenu';
import { useMenu } from '../menu/menuStore';
import { mouseDown, mouseUp } from '../menu/pressTesting';
import { profileDotIcon, ProfileSwitcher } from './ProfileSwitcher';

describe('profile menu icons (K69)', () => {
  it('each profile row shows its colour as a dot; the current one has a ring', () => {
    const Other = profileDotIcon('#2ece9d', false);
    const Current = profileDotIcon('#d90171', true);
    const { container } = render(<><Other size={16} /><Current size={16} /></>);
    const [other, current] = container.querySelectorAll('svg');
    expect(other.querySelector('circle')!.getAttribute('fill')).toBe('#2ece9d');
    expect(other.querySelectorAll('circle')).toHaveLength(1);
    expect(current.querySelector('circle')!.getAttribute('fill')).toBe('#d90171');
    expect(current.querySelectorAll('circle')).toHaveLength(2);
  });

  it('one stable component per colour and state, so rows do not remount their icon', () => {
    expect(profileDotIcon('#4d88ff', false)).toBe(profileDotIcon('#4d88ff', false));
    expect(profileDotIcon('#4d88ff', true)).not.toBe(profileDotIcon('#4d88ff', false));
  });
});

describe('the profile picker opens on the current profile', () => {
  afterEach(() => { cleanup(); act(() => useMenu.getState().close()); });
  it('passes the current profile as the initial row', () => {
    const mk = (id: string) => ({ ...EMPTY_PROFILE, id, name: id, color: '#4d88ff' });
    useAppState.setState({ profiles: [mk('a'), mk('b')], profile: mk('b') });
    render(<ProfileSwitcher />);
    fireEvent.click(screen.getByRole('button', { name: 'Profile: b' }));
    expect(useMenu.getState().initialRow).toBe('profile.b');
  });
});

describe('the profile picker: press, drag onto a profile, release', () => {
  afterEach(() => { cleanup(); act(() => useMenu.getState().close()); });
  const mk = (id: string) => ({ ...EMPTY_PROFILE, id, name: id, color: '#4d88ff' });
  const setUp = () => {
    const switchProfile = vi.fn(async () => {});
    useAppState.setState({ profiles: [mk('a'), mk('b')], profile: mk('a'), switchProfile });
    render(<><ProfileSwitcher /><ContextMenu /></>);
    return { switchProfile, button: screen.getByRole('button', { name: 'Profile: a' }) };
  };

  it('opens on the press; the release on another profile switches to it', () => {
    const { switchProfile, button } = setUp();
    mouseDown(button);
    expect(useMenu.getState().rows).not.toBeNull();
    mouseUp(screen.getByText('b'), 60, 40, document.body);
    expect(switchProfile).toHaveBeenCalledExactlyOnceWith('b');
    expect(useMenu.getState().rows).toBeNull();
  });

  it('released on the button, it stays open (a click); a second click closes it', () => {
    const { switchProfile, button } = setUp();
    mouseDown(button);
    mouseUp(button, 10, 10, button);
    expect(useMenu.getState().rows).not.toBeNull();
    mouseDown(button);
    mouseUp(button, 10, 10, button);
    expect(useMenu.getState().rows).toBeNull();
    expect(switchProfile).not.toHaveBeenCalled();
  });

  it('released on the separator or outside, nothing switches and the menu stays', async () => {
    const { switchProfile, button } = setUp();
    mouseDown(button);
    mouseUp(document.querySelector('.ctx-sep')!, 60, 60, document.body);
    expect(useMenu.getState().rows).not.toBeNull();
    mouseDown(button);
    mouseUp(button, 10, 10, button); // closes it (the toggle)
    await new Promise((r) => setTimeout(r, 0)); // the gesture's end, as between two real clicks
    mouseDown(button);
    mouseUp(document.body, 400, 400, document.body);
    expect(useMenu.getState().rows).not.toBeNull();
    expect(switchProfile).not.toHaveBeenCalled();
  });
});
