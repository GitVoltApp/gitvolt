// @vitest-environment happy-dom
// The tab width setting against real Monaco models: display only, never the text.
import { describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, useAppState } from '../../app/state';
import { followEditorSettings, monaco } from './setup';

vi.mock('../../api/client', () => ({ api: { saveSettings: vi.fn(async () => null), saveProfile: vi.fn(async () => null), openUrl: vi.fn() }, errorMessage: String }));

describe('the tab width setting', () => {
  it("sets every model's tab width from its creation on, keeps its guessed indentation, and never touches its text", () => {
    useAppState.setState({ settings: { ...DEFAULT_SETTINGS, editorTabSize: 8 } });
    followEditorSettings();
    const tabbed = '\tif (x) {\n\t\ty();\n\t}\n';
    const spaced = 'if (x) {\n  y();\n  if (z) {\n    w();\n  }\n}\n\tz;\n';
    const t = monaco.editor.createModel(tabbed, 'plaintext');
    const s = monaco.editor.createModel(spaced, 'plaintext');
    expect(t.getOptions()).toMatchObject({ tabSize: 8, indentSize: 8, insertSpaces: false });
    expect(s.getOptions()).toMatchObject({ tabSize: 8, indentSize: 2, insertSpaces: true });
    useAppState.getState().setSettings({ editorTabSize: 3 });
    expect(t.getOptions()).toMatchObject({ tabSize: 3, indentSize: 3, insertSpaces: false });
    expect(s.getOptions()).toMatchObject({ tabSize: 3, indentSize: 2, insertSpaces: true });
    useAppState.getState().setSettings({ editorTabSize: 99 });
    expect(t.getOptions().tabSize).toBe(16);
    expect([t.getValue(), s.getValue()]).toEqual([tabbed, spaced]);
    t.dispose();
    s.dispose();
  });
});
