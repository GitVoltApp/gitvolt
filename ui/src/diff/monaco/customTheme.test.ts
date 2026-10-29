// @vitest-environment happy-dom
// Custom themes in the real Monaco (#14): Monaco's defineTheme takes only `[a-z0-9-]` names and a
// built-in base, and the editor must still load if it ever refuses one.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as monacoApi from 'monaco-editor/editor/editor.api';
import githubDark from 'shiki/themes/github-dark-default.mjs';
import type { CustomTheme } from '../../api/gen/CustomTheme';
import { customThemeDef } from '../../theme/custom';
import { THEMES } from '../../theme/themes';
import type { Monaco } from './setup';

const highlightClient = vi.fn();
vi.mock('../highlight/client', () => ({ highlightClient }));
vi.mock('../highlight/modelTokens', () => ({ useWorkerTokens: vi.fn(), setTokenTheme: vi.fn() }));

const monaco = monacoApi as unknown as Monaco;

/** A GitHub Dark copy with many colours of its own, in every form a colour field takes. */
const ISSUE_THEME: CustomTheme = {
  id: 'custom-5e1f0a2b', name: 'GitHub Dark (Custom)', base: 'github-dark', graph: [],
  colors: {
    'app-bg0': '#0b0f14', 'panel-bg0': '#10161d', 'panel-bg1': '#161d26', 'text-normal': '#d8dee6',
    'text-secondary': '#8b96a3', blue: '#4c9aff', green: '#3fb950', red: '#f85149', orange: '#d29922',
    yellow: '#e3b341', 'selection-bg': 'rgba(76, 154, 255, 0.35)', 'scroll-thumb-bg': '#ffffff22',
    'scroll-thumb-hover-bg': 'rgb(255, 255, 255)', 'diff-inserted-line': '#3fb95026',
    'diff-removed-line': '#f8514926', 'diff-inserted-text': '#3fb95055', 'diff-removed-text': '#f8514955',
    'diff-diagonal-fill': '#30363d', 'toolbar-bg0': '#0d1117', 'tab-bar-bg': '#0d1117', purple: '#a371f7',
  },
};
const client = {
  themes: [{ name: 'github-dark-default', theme: githubDark, colors: ['#E6EDF3', '#0D1117', '#FF7B72'] }],
  loadLanguage: vi.fn(),
};

/** The colour Monaco's current theme gives `key`, from the CSS variables it writes. */
const themeColour = (key: string) => {
  const css = [...document.querySelectorAll('style')].map((s) => s.textContent).join('\n');
  return css.match(new RegExp(`--vscode-${key.replace('.', '-')}: (#[0-9a-f]+)`, 'i'))?.[1].toLowerCase();
};

describe('a custom editor theme in Monaco', () => {
  beforeEach(() => {
    vi.resetModules();
    highlightClient.mockReset().mockResolvedValue(client);
  });
  afterEach(() => vi.restoreAllMocks());

  it("defines and applies the issue's theme (GitHub Dark with many colours) without throwing", async () => {
    const { useTheme } = await import('../../theme/store');
    const { ensureTheme } = await import('./shiki');
    const { currentEditorTheme } = await import('../../theme/editorThemes');
    useTheme.getState().set(customThemeDef(ISSUE_THEME));
    const setTheme = vi.spyOn(monaco.editor, 'setTheme');
    const warn = vi.spyOn(console, 'warn');
    await expect(ensureTheme(monaco)).resolves.toBeUndefined();
    expect(currentEditorTheme()).toMatch(/^github-dark-default--[0-9a-z]+$/);
    expect(setTheme).toHaveBeenLastCalledWith(currentEditorTheme());
    expect(warn).not.toHaveBeenCalled();
    // Its own colours, over its base's tokens.
    const ed = monaco.editor.create(document.createElement('div'));
    expect(themeColour('editor.background')).toBe('#0b0f14');
    ed.dispose();
  });

  it("falls back to the base's editor theme when Monaco refuses the custom one, and logs why", async () => {
    const { useTheme } = await import('../../theme/store');
    const { appEditorTheme, ensureTheme, setAppEditorTheme } = await import('./shiki');
    useTheme.getState().set(customThemeDef(ISSUE_THEME));
    const define = monaco.editor.defineTheme.bind(monaco.editor);
    vi.spyOn(monaco.editor, 'defineTheme').mockImplementation((name, data) => {
      if (name.includes('--')) throw new Error('Illegal theme name!');
      define(name, data);
    });
    const realSet = monaco.editor.setTheme.bind(monaco.editor);
    const setTheme = vi.spyOn(monaco.editor, 'setTheme');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await expect(ensureTheme(monaco)).resolves.toBeUndefined();
    expect(setTheme).toHaveBeenLastCalledWith('github-dark-default');
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('using github-dark-default'), expect.objectContaining({ message: 'Illegal theme name!' }));
    // A new editor gets the base's too, and the refused name isn't tried (or logged) again.
    expect(appEditorTheme(monaco)).toBe('github-dark-default');
    expect(warn).toHaveBeenCalledTimes(1);

    // A setTheme that throws for a defined custom theme gives way to the base's, too.
    vi.mocked(monaco.editor.defineTheme).mockImplementation(define);
    useTheme.getState().set(customThemeDef({ ...ISSUE_THEME, colors: { 'app-bg0': '#101010' } }));
    setTheme.mockImplementation((name) => {
      if (name.includes('--')) throw new Error('bad colour');
      realSet(name);
    });
    expect(() => setAppEditorTheme(monaco)).not.toThrow();
    expect(setTheme).toHaveBeenLastCalledWith('github-dark-default');
    expect(appEditorTheme(monaco)).toBe('github-dark-default');
    // A built-in that throws still throws: there's nothing to fall back to.
    useTheme.getState().set(THEMES['github-dark']);
    setTheme.mockImplementation(() => { throw new Error('broken'); });
    expect(() => setAppEditorTheme(monaco)).toThrow('broken');
  });
});
