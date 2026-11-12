// First: where actions start (the confirm model), seen before the key router or React acts.
import './ui/arm/origin';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './theme/tokens.css';
import { App } from './App';
import { installBusyPageDetector } from './debug/busyPage';
import { bindDebugLogging, installFrontendErrorLogging } from './debug/frontendLog';
import { bindThemeToSettings } from './theme/bind';
import { installEditorSettingsMigration } from './diff/editorSettings';
import { installFontZoom } from './diff/fontZoom';
import { registerEditorFont } from './theme/editorFont';
import { bindFontsToSettings } from './theme/fonts';
import { installZoom } from './ui/zoom';
import { applyPageZoom } from './demo/pageZoom';

// First, before anything else can throw: uncaught errors reach the backend log file (spec §16.2).
installFrontendErrorLogging();
bindDebugLogging();
// A page that keeps the main thread busy for nothing writes what ran to the log file. Not in the
// website demo: it has no log file.
if (import.meta.env.MODE !== 'demo') {
  const busy = installBusyPageDetector();
  if (import.meta.env.DEV || import.meta.env.MODE === 'e2e') window.__gbBusy = busy;
}

// Before the first render: the saved zoom applies from the start (spec §12.2), and so does the
// last saved theme (its localStorage mirror, R3) until the settings load and take over. The
// website demo has no webview to zoom: it zooms the page itself (demo/pageZoom.ts).
installZoom(import.meta.env.MODE === 'demo' ? applyPageZoom : undefined);
// Over a file's or a diff's body, the zoom keys and Ctrl+wheel size its text instead.
installFontZoom();
bindThemeToSettings();
// The UI and reading fonts (the UI font's mirror until the settings load, as the theme's).
bindFontsToSettings();
installEditorSettingsMigration();
// The editors' bundled font (JetBrains Mono), for Monaco and the rendered Markdown's code alike.
registerEditorFont();

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);
