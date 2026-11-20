// Monaco's deleted-lines zones (Inline and Hunk mode), made drawable lazily: three small rewrites
// of its DiffEditorViewZones, which src/diff/monaco/deletedZone.ts fills in. Monaco draws every
// old line of a change up front and has no setting against it; a deleted 20,000-line file was
// 240,000 elements. Each rewrite must match exactly once: a Monaco upgrade that changes the code
// fails the build (and monacoDeletedZones.test.ts) instead of quietly drawing every line again.
import type { Plugin } from 'vite';

const FILE = /[\\/]monaco-editor[\\/]esm[\\/]vs[\\/]editor[\\/]browser[\\/]widget[\\/]diffEditor[\\/]components[\\/]diffEditorViewZones[\\/]diffEditorViewZones\.js$/;

const REWRITES: [from: string, to: string][] = [
  // The zone's lines: the hook's renderer when one is set (it returns a result of the same shape).
  [
    'const result = renderLines(source, renderOptions, decorations, deletedCodeDomNode);',
    'const result = deletedZoneHook.render ? deletedZoneHook.render(source, renderOptions, decorations, deletedCodeDomNode) : renderLines(source, renderOptions, decorations, deletedCodeDomNode);',
  ],
  // Its margin's − signs: a lazy result draws them with its lines.
  [
    'if (this._options.renderIndicators.read(reader)) {',
    'if (result.attachMargin) {\n                            result.attachMargin(marginDomNode, this._options.renderIndicators.read(reader));\n                        }\n                        else if (this._options.renderIndicators.read(reader)) {',
  ],
  // The zone's top on every render, scrolling included: where a lazy result draws.
  ['setZoneId(id) { zoneId = id; },', 'setZoneId(id) { zoneId = id; },\n                            onDomNodeTop: result.onDomNodeTop,'],
];

/** The rewritten DiffEditorViewZones module; throws when a rewrite doesn't match exactly once. */
export function rewriteDeletedZones(code: string): string {
  for (const [from, to] of REWRITES) {
    const n = code.split(from).length - 1;
    if (n !== 1) throw new Error(`monacoDeletedZones: expected one \`${from}\` in Monaco's diffEditorViewZones.js, found ${n}`);
    code = code.replace(from, () => to);
  }
  return `${code}\nconst deletedZoneHook = { render: undefined };\nexport { deletedZoneHook };\n`;
}

const transform = (code: string, id: string) => (FILE.test(id.split('?')[0]!) ? { code: rewriteDeletedZones(code), map: null } : null);

/** For the build and the dev server's own transforms. */
export const monacoDeletedZones = (): Plugin => ({ name: 'gitvolt-monaco-deleted-zones', transform });
/** For the dev server's dependency pre-bundling (optimizeDeps), where Monaco is bundled. */
export const monacoDeletedZonesDeps = () => ({ name: 'gitvolt-monaco-deleted-zones', transform });
