import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { useAppState } from '../../app/state';
import { autoLineHeight, clampEditorFont, clampLineHeight, editorFontFamily } from '../../diff/options';
import type { CodeTokens } from '../../diff/highlight/protocol';
import { clampReadingFont, clampReadingLineHeight, DEFAULT_UI_FONT, proseFontFamily, readingFontFamily } from '../../theme/fontSettings';
import { useFontFamilies } from '../../theme/fontList';
import { useTheme } from '../../theme/store';
import { fontChoice, FontPicker, type FontRole } from './FontPicker';
import { TextField } from './fields';

/** One line of other scripts and diacritics in every preview: what a font lacks shows at once. */
export const MIXED_SCRIPTS = 'Café, naïve, Straße, Tiếng Việt, Ελληνικά, Кириллица, 日本語';

/** The editor preview's code: a comment, a string, numbers, a function, the look-alikes 0O 1lI,
 * operators that ligature fonts join (=> != === >= ->) and braces. */
export const CODE_SAMPLE = [
  `// ${MIXED_SCRIPTS}`,
  'function scale(values: number[], factor = 1.5): number[] {',
  '  const label = "The quick brown fox 0O 1lI"; // -> label',
  '  if (values.length >= 10 && factor !== 0) return values;',
  '  return values.map((v) => (v === 42 ? v : v * factor));',
  '}',
].join('\n');

/**
 * A font setting's control: the picker, the box for a hand-written CSS list when that's what's set
 * (or "Custom font list…" asked for it), and the preview panel under the label, captioned. The panel
 * shows the family under the picker's pointer or arrows, else what's typed in the box, else the
 * setting; the open list keeps clear of it, so what's under the pointer shows. Only a picture: one
 * image to assistive tech, nothing to focus, press or select.
 */
export function FontSetting({ role, placeholder, preview }: { role: FontRole; placeholder: string; preview(value: string): ReactNode }) {
  const value = useAppState((s) => s.settings[role.id as 'editorFontFamily' | 'readingFontFamily' | 'uiFontFamily']);
  const list = useFontFamilies((s) => s.list);
  const [hover, setHover] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [asked, setAsked] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const custom = asked || fontChoice(value, list).kind === 'custom';
  const save = (v: string) => {
    setDraft(null);
    useAppState.getState().setSettings({ [role.id]: v });
  };
  // Back to the default (↺, Reset all): the box goes with it.
  useEffect(() => { if (!value) setAsked(false); }, [value]);
  useEffect(() => { if (asked) document.getElementById(`input-${role.id}-custom`)?.focus({ preventScroll: true }); }, [asked, role.id]);
  return (
    <>
      <div className="font-setting">
        <FontPicker role={role} value={value} avoid={panel} onPick={(v) => { setAsked(false); save(v); }} onPreview={setHover} onCustom={() => setAsked(true)} />
        {custom && <TextField id={`${role.id}-custom`} label="Custom font list" value={value} placeholder={placeholder} onDraft={setDraft} onCommit={save} />}
      </div>
      <div ref={panel} className="font-preview-box" role="img" aria-label={`Preview of the ${role.noun} font`}>
        <div className="font-preview-caption" aria-hidden>Preview</div>
        {preview(hover ?? draft ?? value)}
      </div>
    </>
  );
}

const tokenStyle = (t: { color?: string; fontStyle?: number }): CSSProperties => ({
  color: t.color,
  fontStyle: t.fontStyle && t.fontStyle & 1 ? 'italic' : undefined,
  fontWeight: t.fontStyle && t.fontStyle & 2 ? 'bold' : undefined,
});

/** The sample's tokens in the editor theme, from the highlight worker (plain until they come). */
function useSampleTokens(): CodeTokens | null {
  const theme = useTheme((s) => s.id);
  const [tokens, setTokens] = useState<CodeTokens | null>(null);
  useEffect(() => {
    let live = true;
    let cancel = () => {};
    // Imported when shown: the highlighter's languages stay out of the startup bundle.
    void import('../../diff/highlight/codeTokens').then(({ codeTokens }) => {
      if (!live) return;
      const job = codeTokens(CODE_SAMPLE, 'typescript');
      cancel = job.cancel;
      job.result.then((t) => { if (live) setTokens(t); }, () => {});
    }, () => {});
    return () => { live = false; cancel(); };
  }, [theme]);
  return tokens;
}

/** The editor's font at its size, line height and ligatures, in the editor theme's colours. Its
 * height is the lines' (whatever the font), so a family change never moves the page. */
export function EditorFontPreview({ family }: { family: string }) {
  const s = useAppState((st) => st.settings);
  const tokens = useSampleTokens();
  const size = clampEditorFont(s.editorFontSize);
  const set = clampLineHeight(s.editorLineHeight);
  const line = set ? Math.max(set, size) : autoLineHeight(size);
  const lines: CodeTokens['lines'] = tokens?.lines ?? CODE_SAMPLE.split('\n').map((content) => [{ content }]);
  const style: CSSProperties = {
    fontFamily: editorFontFamily(family), fontSize: size, lineHeight: `${line}px`, height: lines.length * line + 16,
    fontFeatureSettings: s.editorFontLigatures ? '"liga" on, "calt" on' : '"liga" off, "calt" off',
    color: tokens?.fg, background: tokens?.bg,
  };
  return (
    <pre className="font-preview font-preview-code" data-testid="editorFontFamily-preview" aria-hidden style={style}>
      {lines.map((l, i) => <div key={i}>{l.length ? l.map((t, j) => <span key={j} style={tokenStyle(t)}>{t.content}</span>) : ' '}</div>)}
    </pre>
  );
}

/** Rendered Markdown's prose at the reading size and line height, its code in the editor font. Cut
 * at a fixed height for the size: a font that wraps the paragraph differently can't move the page. */
export function ReadingFontPreview({ family }: { family: string }) {
  const s = useAppState((st) => st.settings);
  const size = clampReadingFont(s.readingFontSize);
  const lh = clampReadingLineHeight(s.readingLineHeight);
  const style = {
    fontFamily: readingFontFamily(family), fontSize: size, lineHeight: lh,
    height: Math.round(16 + size * (1.3 * 1.25 + 0.3) + size * lh * 5 + size * 0.4 * 3),
    '--md-code-font': editorFontFamily(s.editorFontFamily),
  } as CSSProperties;
  return (
    <div className="font-preview font-preview-prose" data-testid="readingFontFamily-preview" aria-hidden style={style}>
      <h4>Release notes</h4>
      <p>The quick brown fox jumps over the lazy dog. READMEs, descriptions and comments show in this font, wherever Markdown is rendered.</p>
      <p>Call <code style={{ fontVariantLigatures: s.editorFontLigatures ? 'normal' : 'none' }}>scale(values, 1.5)</code> to resize every value.</p>
      <p>{MIXED_SCRIPTS}</p>
    </div>
  );
}

/** Plain lines in the UI font at the sizes the app draws: a heading, body text, a small secondary
 * line, and the mixed-script line. Words only, nothing shaped like a control. */
export function UiFontPreview({ family }: { family: string }) {
  return (
    <div className="font-preview font-preview-ui" data-testid="uiFontFamily-preview" aria-hidden style={{ fontFamily: proseFontFamily(family) || DEFAULT_UI_FONT, lineHeight: 1.4, height: 100 }}>
      <div className="font-preview-title">Sphinx of black quartz, judge my vow</div>
      <div>The quick brown fox jumps over the lazy dog. 0123456789</div>
      <div className="font-preview-small">How vexingly quick daft zebras jump</div>
      <div>{MIXED_SCRIPTS}</div>
    </div>
  );
}
