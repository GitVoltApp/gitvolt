// @vitest-environment happy-dom
import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppState } from '../app/state';
import { useToast } from '../ui/toastStore';
import { fontZoomOf, installZoom, useZoom } from '../ui/zoom';
import { installFontZoom, nextEditorFont, nextReadingFont, readingFontVar, useReadingFontPx } from './fontZoom';

vi.mock('../api/client', () => ({ api: { saveSettings: vi.fn(async () => {}), saveProfile: vi.fn(async () => {}) }, errorMessage: String, onEvent: () => () => {} }));

const key = (k: string) => new KeyboardEvent('keydown', { key: k, ctrlKey: true, bubbles: true, cancelable: true });
const wheel = (deltaY: number, ctrlKey = true, deltaMode = 0) => new WheelEvent('wheel', { deltaY, ctrlKey, deltaMode, bubbles: true, cancelable: true });
const font = () => useAppState.getState().settings.editorFontSize;
const reading = () => useAppState.getState().settings.readingFontSize;

describe('nextEditorFont, nextReadingFont', () => {
  it('step 1 px in or out, clamped to 8-32; reset is 13 (Monaco\'s size, and the reading text\'s default)', () => {
    expect(nextEditorFont(13, 1)).toBe(14);
    expect(nextEditorFont(13, -1)).toBe(12);
    expect(nextEditorFont(32, 1)).toBe(32);
    expect(nextEditorFont(8, -1)).toBe(8);
    expect(nextEditorFont(20, 0)).toBe(13);
    expect(nextEditorFont(Number.NaN, 1)).toBe(14);
    expect(nextReadingFont(15, 1)).toBe(16);
    expect(nextReadingFont(8, -1)).toBe(8);
    expect(nextReadingFont(40, 1)).toBe(32);
    expect(nextReadingFont(20, 0)).toBe(13);
  });
});

describe('the rendered Markdown font size', () => {
  it('follows readingFontSize, clamped, as the panes\' --md-font-size; their code takes the editor\'s font', () => {
    expect(readingFontVar(15)).toEqual({ '--md-font-size': '15px' });
    expect(readingFontVar(99)).toEqual({ '--md-font-size': '32px' });
    // The panes' code takes the editor's font family (its default stack when empty) and ligatures.
    expect(readingFontVar(13, { family: '', ligatures: false })).toEqual({ '--md-font-size': '13px', '--md-code-font': "'JetBrains Mono', 'Noto Sans Mono', 'Ubuntu Sans Mono', 'Liberation Mono', monospace", '--md-code-ligatures': 'none' });
    expect(readingFontVar(13, { family: 'Iosevka', ligatures: true })).toMatchObject({ '--md-code-font': '"Iosevka", "JetBrains Mono", "Noto Sans Mono", "Ubuntu Sans Mono", "Liberation Mono", monospace', '--md-code-ligatures': 'normal' });
    // By default it's the size the panes had (the editor's 13), whatever the editor's size now.
    const { result, rerender } = renderHook(() => useReadingFontPx());
    expect(result.current).toBe(13);
    useAppState.getState().setSettings({ editorFontSize: 17, readingFontSize: 16 });
    rerender();
    expect(result.current).toBe(16);
    useAppState.getState().setSettings({ editorFontSize: 13, readingFontSize: 13 });
  });
});

describe('fontZoomOf', () => {
  it('names the innermost marked box\'s size: a rendered pane\'s reading text, else code; null outside', () => {
    document.body.innerHTML = '<div data-font-zoom id="code"><div data-font-zoom="reading" id="md"><p id="p">x</p><div data-font-zoom id="nested"></div></div></div><div id="out"></div>';
    const at = (id: string) => fontZoomOf(document.getElementById(id));
    expect(at('code')).toBe('code');
    expect(at('md')).toBe('reading');
    expect(at('p')).toBe('reading');
    expect(at('nested')).toBe('code');
    expect(at('out')).toBeNull();
    expect(fontZoomOf(null)).toBeNull();
    document.body.innerHTML = '';
  });
});

describe('installFontZoom', () => {
  let off: (() => void)[] = [];
  let panel: HTMLElement;
  let inner: HTMLElement;
  let prose: HTMLElement;
  let outside: HTMLElement;
  beforeEach(() => {
    useAppState.getState().setSettings({ editorFontSize: 13, readingFontSize: 13 });
    useToast.setState({ message: null });
    // An editor's box, a rendered Markdown pane's (a diff shows one or the other), and the rest.
    document.body.innerHTML = '<div data-font-zoom><div tabindex="-1" id="inner">text</div></div><div data-font-zoom="reading"><p tabindex="-1" id="prose">prose</p></div><div tabindex="-1" id="outside">other</div>';
    panel = document.querySelector('[data-font-zoom]')!;
    inner = document.getElementById('inner')!;
    prose = document.getElementById('prose')!;
    outside = document.getElementById('outside')!;
    off = [installZoom(() => {}), installFontZoom()];
  });
  afterEach(() => {
    for (const f of off) f();
    document.body.innerHTML = '';
  });

  it('Ctrl+= / Ctrl+- / Ctrl+0 in an editor change the code size, not the app zoom, and say so', () => {
    const zoom = useZoom.getState().zoom;
    inner.dispatchEvent(key('='));
    inner.dispatchEvent(key('='));
    expect(font()).toBe(15);
    expect(useToast.getState().message).toBe('Code text 15 px');
    inner.dispatchEvent(key('-'));
    expect(font()).toBe(14);
    inner.dispatchEvent(key('0'));
    expect(font()).toBe(13);
    expect(reading()).toBe(13);
    expect(useZoom.getState().zoom).toBe(zoom);
    expect(panel).toBeTruthy();
  });

  it('in a rendered Markdown pane they change the reading size instead, and Ctrl+0 resets it', () => {
    const zoom = useZoom.getState().zoom;
    prose.dispatchEvent(key('='));
    prose.dispatchEvent(key('='));
    expect(reading()).toBe(15);
    expect(font()).toBe(13);
    expect(useToast.getState().message).toBe('Reading text 15 px');
    prose.dispatchEvent(key('0'));
    expect(reading()).toBe(13);
    expect(useZoom.getState().zoom).toBe(zoom);
  });

  it('outside the panes, the keys still zoom the app', () => {
    const zoom = useZoom.getState().zoom;
    outside.dispatchEvent(key('='));
    expect(useZoom.getState().zoom).toBeGreaterThan(zoom);
    expect(font()).toBe(13);
    expect(reading()).toBe(13);
    outside.dispatchEvent(key('0'));
  });

  it('Ctrl+wheel over the panel steps the size once per notch; a touchpad\'s small deltas add up', () => {
    const e = wheel(-100);
    inner.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    expect(font()).toBe(14);
    inner.dispatchEvent(wheel(100));
    inner.dispatchEvent(wheel(100));
    expect(font()).toBe(12);
    for (let i = 0; i < 4; i++) inner.dispatchEvent(wheel(-10));
    expect(font()).toBe(12);
    for (let i = 0; i < 2; i++) inner.dispatchEvent(wheel(-10));
    expect(font()).toBe(13);
    // Lines, not pixels (Firefox's mode 1): one line is a step too.
    inner.dispatchEvent(wheel(-3, true, 1));
    expect(font()).toBe(14);
  });

  it('Ctrl+wheel over a rendered pane sizes its reading text', () => {
    prose.dispatchEvent(wheel(-100));
    expect(reading()).toBe(14);
    expect(font()).toBe(13);
    expect(useToast.getState().message).toBe('Reading text 14 px');
  });

  it('a plain wheel, or Ctrl+wheel elsewhere, leaves the size alone', () => {
    const plain = wheel(-100, false);
    inner.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
    outside.dispatchEvent(wheel(-100));
    expect(font()).toBe(13);
  });
});
