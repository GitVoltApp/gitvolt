// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { THEME_STORAGE_KEY, writeThemeMirror } from './apply';
import { customThemeDef } from './custom';
import { THEMES } from './themes';

// index.html's inline `theme-first-paint` script (review I1): it must paint the mirrored theme's
// background and colour scheme synchronously, before any stylesheet or module script.
const html = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'index.html'), 'utf8');
const script = html.match(/<script id="theme-first-paint">([\s\S]*?)<\/script>/)?.[1] ?? '';
const run = () => new Function(script)();
const root = document.documentElement;

describe("index.html's first-paint theme script", () => {
  beforeEach(() => {
    localStorage.clear();
    root.removeAttribute('style');
    delete root.dataset.theme;
  });
  afterEach(() => {
    root.removeAttribute('style');
    delete root.dataset.theme;
  });

  it('is an inline classic script in <head>, ahead of every stylesheet and the module entry', () => {
    expect(script).not.toBe('');
    const head = html.slice(0, html.indexOf('</head>'));
    expect(head).toContain('<script id="theme-first-paint">');
    const at = html.indexOf('<script id="theme-first-paint">');
    for (const later of ['<link', '<style', '<script type="module"']) {
      const i = html.indexOf(later);
      if (i >= 0) expect(i, later).toBeGreaterThan(at);
    }
    expect(script).toContain(THEME_STORAGE_KEY);
  });

  it('applies a stored light theme to the root synchronously', () => {
    writeThemeMirror(THEMES.light);
    run();
    expect(root.dataset.theme).toBe('light');
    expect(root.style.colorScheme).toBe('light');
    expect(root.style.getPropertyValue('--app-bg0')).toBe(THEMES.light.colors['app-bg0']);
  });

  it('applies a dark one the same way', () => {
    writeThemeMirror(THEMES['solarized-dark']);
    run();
    expect(root.style.colorScheme).toBe('dark');
    expect(root.style.getPropertyValue('--app-bg0')).toBe('#002b36');
  });

  it("paints a mirrored custom theme's own background and its base's kind", () => {
    const custom = { id: 'custom-0a1b2c3d', name: 'Light (Custom)', base: 'light', colors: { 'app-bg0': '#fafaf0' }, graph: [] };
    writeThemeMirror(customThemeDef(custom), custom);
    run();
    expect(root.dataset.theme).toBe('custom-0a1b2c3d');
    expect(root.style.colorScheme).toBe('light');
    expect(root.style.getPropertyValue('--app-bg0')).toBe('#fafaf0');
  });

  it('leaves the root alone with no mirror, a corrupt one, or a bad background, and never throws', () => {
    run();
    expect(root.getAttribute('style')).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, '{nope');
    expect(run).not.toThrow();
    expect(root.getAttribute('style')).toBeNull();
    localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ id: 'x', kind: 'sepia', bg: 'url(evil)' }));
    run();
    expect(root.style.getPropertyValue('--app-bg0')).toBe('');
    expect(root.style.colorScheme).toBe('');
  });

  it('declares no globals', () => {
    writeThemeMirror(THEMES.nord);
    run();
    expect('m' in window).toBe(false);
    expect('r' in window).toBe(false);
  });
});
