import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { rewriteDeletedZones } from './monacoDeletedZones';

const file = fileURLToPath(new URL('../node_modules/monaco-editor/esm/vs/editor/browser/widget/diffEditor/components/diffEditorViewZones/diffEditorViewZones.js', import.meta.url));

// If this fails after a Monaco upgrade, the code the rewrite hooks into moved: see
// src/diff/monaco/deletedZone.ts for what each rewrite is for.
describe("the rewrite of Monaco's deleted-lines zones", () => {
  it('applies to the installed Monaco: the hook, the margin and the zone top, once each', () => {
    const out = rewriteDeletedZones(readFileSync(file, 'utf8'));
    expect(out.match(/deletedZoneHook\.render \?/g)).toHaveLength(1);
    expect(out.match(/result\.attachMargin\(marginDomNode, /g)).toHaveLength(1);
    expect(out.match(/onDomNodeTop: result\.onDomNodeTop,/g)).toHaveLength(1);
    expect(out).toMatch(/export \{ deletedZoneHook \};\n$/);
  });

  it("refuses a Monaco whose code doesn't match", () => {
    const code = readFileSync(file, 'utf8');
    expect(() => rewriteDeletedZones(code.replace('setZoneId(id) { zoneId = id; },', 'setZoneId(id) { zoneId = id },'))).toThrow(/setZoneId/);
  });
});
