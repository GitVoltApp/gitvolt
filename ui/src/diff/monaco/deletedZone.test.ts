// @vitest-environment happy-dom
// The lazy deleted-lines zone against the installed Monaco's renderLines and models: what it
// draws for a scroll position, its margin's − signs, its old positions (Monaco's copy of a
// selection reads them), and what it keeps while text in it is selected.
import { describe, expect, it } from 'vitest';
import * as monaco from 'monaco-editor/editor/editor.api';
import { LineSource, type LineTokensLike, type RenderOptionsLike } from 'monaco-editor/editor/browser/widget/diffEditor/components/diffEditorViewZones/renderLines';
import { Range } from 'monaco-editor/editor/common/core/range';
import { InlineDecoration } from 'monaco-editor/editor/common/viewModel/inlineDecorations';
import { lazyZoneRows, rowOfElement } from './deletedRows';
import { LazyDeletedLines, renderDeletedLines } from './deletedZone';

const LH = 19;
// What renderLines reads of RenderOptions.fromEditor's: a 13 px monospace font, 19 px lines.
const fontInfo = {
  getMassagedFontFamily: () => 'monospace', fontWeight: 'normal', fontSize: 13, fontFeatureSettings: '', fontVariationSettings: '', lineHeight: LH, letterSpacing: 0,
  isMonospace: true, canUseHalfwidthRightwardsArrow: true, spaceWidth: 8, middotWidth: 8, wsmiddotWidth: 8, typicalHalfwidthCharacterWidth: 8,
};
const options = {
  tabSize: 4, fontInfo, disableMonospaceOptimizations: false, typicalHalfwidthCharacterWidth: 8, scrollBeyondLastColumn: 4, lineHeight: LH, lineDecorationsWidth: 18,
  stopRenderingLineAfter: 10_000, renderWhitespace: 'none', renderControlCharacters: false, fontLigatures: 'off', verticalScrollbarSize: 10, useTwoCellFullwidthCharacters: false, setWidth: true,
} as RenderOptionsLike;

/** A deleted block of `n` old lines, `removedLine 1` … (the 7th one longer), as Monaco hands it over. */
function block(n: number, decorations: InlineDecoration[] = []) {
  const model = monaco.editor.createModel(Array.from({ length: n }, (_, i) => `removedLine ${i + 1}${i === 6 ? ' and a longer tail' : ''}`).join('\n'), 'plaintext');
  // The model's tokens, as Monaco reads them for the zone (`tokenization` isn't in the public API).
  const tokens = (model as unknown as { tokenization: { getLineTokens(n: number): LineTokensLike } }).tokenization;
  const source = new LineSource(Array.from({ length: n }, (_, i) => tokens.getLineTokens(i + 1)), [], false, false);
  const dom = document.createElement('div');
  const margin = document.createElement('div');
  margin.appendChild(Object.assign(document.createElement('div'), { className: 'lightbulb-glyph' }));
  document.body.appendChild(dom);
  const zone = renderDeletedLines(source, options, decorations, dom);
  return { zone, dom, margin };
}
const lazy = (n: number, decorations?: InlineDecoration[]) => {
  const b = block(n, decorations);
  expect(b.zone).toBeInstanceOf(LazyDeletedLines);
  const zone = b.zone as LazyDeletedLines;
  zone.attachMargin(b.margin, true);
  // Monaco marks a zone it shows before it reports its top.
  b.dom.setAttribute('monaco-visible-view-zone', 'true');
  return { ...b, zone };
};
const texts = (el: Element) => [...el.querySelectorAll('.view-line')].map((l) => l.textContent?.replace(/ /g, ' '));

describe('a large deleted block, drawn lazily', () => {
  it("has every row's height and the widest line's width, and draws nothing until Monaco places it", () => {
    const { zone, dom } = lazy(1000);
    expect(zone.heightInLines).toBe(1000);
    expect(zone.viewLineCounts).toHaveLength(1000);
    // `removedLine 7 and a longer tail` (31 columns) and Monaco's 4 past the end, 8 px each.
    expect(zone.minWidthInPx).toBe((31 + 4) * 8);
    expect(dom.querySelectorAll('.view-line')).toHaveLength(0);
    expect(dom.style.lineHeight).toBe(`${LH}px`);
    expect(lazyZoneRows(dom)?.rows).toBe(1000);
  });

  it('draws the chunks on and near the screen as it scrolls, at their place in the zone, and drops the rest', () => {
    const { zone, dom, margin } = lazy(1000);
    zone.onDomNodeTop(0);
    expect(zone.drawnChunks).toEqual([0]);
    expect(texts(dom)).toHaveLength(100);
    expect(texts(dom)[0]).toBe('removedLine 1');
    // Its − signs, one per row, ahead of Monaco's lightbulb.
    expect(margin.querySelectorAll('.delete-sign.codicon-diff-remove')).toHaveLength(100);
    expect(margin.lastElementChild?.className).toBe('lightbulb-glyph');

    // 500 rows down (the zone's top 9,500 px above the viewport's).
    zone.onDomNodeTop(-500 * LH);
    expect(zone.drawnChunks).toEqual([4, 5]);
    const chunk5 = [...dom.children].find((c) => (c as HTMLElement).dataset.firstRow === '500') as HTMLElement;
    expect(chunk5.style.top).toBe(`${500 * LH}px`);
    expect(texts(chunk5)[0]).toBe('removedLine 501');
    expect(margin.querySelectorAll('.delete-sign')).toHaveLength(zone.drawnChunks.length * 100);
    expect(rowOfElement(chunk5.querySelectorAll('.view-line')[2]!.firstElementChild!)).toBe(502);

    // Scrolled out of view: Monaco unmarks it and reports a top far above.
    dom.removeAttribute('monaco-visible-view-zone');
    zone.onDomNodeTop(-1e6);
    expect(zone.drawnChunks).toEqual([]);
    expect(dom.children).toHaveLength(0);
    expect(margin.querySelectorAll('.delete-sign')).toHaveLength(0);
  });

  it('gives the old position of a point in a drawn line, and keeps the chunk a selection starts in', () => {
    const { zone, dom } = lazy(1000);
    zone.onDomNodeTop(-300 * LH);
    const line = [...dom.querySelectorAll('.view-line')].find((l) => l.textContent?.replace(/ /g, ' ') === 'removedLine 305')!;
    const span = line.querySelector('span span')!;
    expect(zone.getModelPositionAt(span, 3)).toMatchObject({ lineNumber: 305, column: 4 });
    expect(zone.getModelPositionAt(dom, 0)).toBeUndefined();

    const sel = document.getSelection()!;
    sel.setBaseAndExtent(span.firstChild!, 0, span.firstChild!, 5);
    zone.onDomNodeTop(-900 * LH);
    expect(zone.drawnChunks).toContain(3);
    expect(zone.drawnChunks).toContain(9);
    sel.removeAllRanges();
    zone.onDomNodeTop(-0.5 * LH);
    expect(zone.drawnChunks).toEqual([0]);
  });

  it("keeps renderLines' marks: whole lines deleted, or only the changed text when the block has inner changes", () => {
    const whole = lazy(500);
    whole.zone.onDomNodeTop(-300 * LH);
    expect(whole.dom.querySelectorAll('.view-line.char-delete').length).toBeGreaterThan(0);
    // An inner change on the block's line 305 only: no line is marked whole, in its chunk or another.
    const inner = lazy(500, [new InlineDecoration(new Range(305, 1, 305, 5), 'char-delete', 0)]);
    inner.zone.onDomNodeTop(-300 * LH);
    expect(inner.dom.querySelectorAll('.view-line').length).toBeGreaterThan(100);
    expect(inner.dom.querySelectorAll('.view-line.char-delete')).toHaveLength(0);
    const marked = [...inner.dom.querySelectorAll('.view-line')].find((l) => l.textContent?.replace(/ /g, ' ') === 'removedLine 305')!;
    expect(marked.querySelector('.char-delete')?.textContent).toBe('remo');
  });

  it('leaves a smaller block to Monaco: every line drawn at once', () => {
    const { zone, dom } = block(50);
    expect(zone).not.toBeInstanceOf(LazyDeletedLines);
    expect(texts(dom)).toHaveLength(50);
    expect(lazyZoneRows(dom)).toBeUndefined();
  });
});
