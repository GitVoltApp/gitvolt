// Large deleted blocks in Inline and Hunk mode, drawn lazily. Monaco draws a change's old lines
// in a view zone, every line up front (DOM for each line and its − sign): a deleted 20,000-line
// file was 240,000 elements and 3.5 s of main-thread work to open, all of it drawn again when the
// file's highlighting arrived. A block of LAZY_MIN_LINES or more is drawn here instead, a chunk of
// lines at a time, only the chunks on or near the screen. The zone keeps its full height, so
// scrolling, the old line numbers beside it and every position stay as they were; the lines look
// as Monaco's, drawn by its own renderLines.
//
// Monaco has no setting for this. build/monacoDeletedZones.ts rewrites its call to renderLines
// into a call to the hook set here, hands the zone our onDomNodeTop and the margin to fill. Its
// test fails when an upgrade changes the code it rewrites.
import * as viewZonesModule from 'monaco-editor/editor/browser/widget/diffEditor/components/diffEditorViewZones/diffEditorViewZones';
import { LineSource, renderLines, type LineSourceLike, type PositionLike, type RenderLinesResultLike, type RenderOptionsLike } from 'monaco-editor/editor/browser/widget/diffEditor/components/diffEditorViewZones/renderLines';
import { applyFontInfo } from 'monaco-editor/editor/browser/config/domFontInfo';
import { Range } from 'monaco-editor/editor/common/core/range';
import { InlineDecoration } from 'monaco-editor/editor/common/viewModel/inlineDecorations';
import { chunksNear, LAZY_MIN_LINES, registerLazyZone, widestColumns, ZoneRows } from './deletedRows';

type Decoration = InlineDecoration;

/** Monaco's − sign in the margin of a deleted line (its `diffRemoveIcon`). */
const SIGN_CLASS = 'delete-sign codicon codicon-diff-remove';

interface Drawn { el: HTMLElement; margin: HTMLElement | null; result: RenderLinesResultLike; start: number }

/**
 * What Monaco's renderLines returns, for a block drawn lazily: the zone's size (all its rows), and
 * the hooks the rewritten Monaco calls. `onDomNodeTop` is the zone's: Monaco calls it on every
 * render with the zone's top in the editor's viewport (scrolling included), so the chunks it draws
 * are on screen in the same frame.
 */
export class LazyDeletedLines {
  readonly rows: ZoneRows;
  readonly heightInLines: number;
  readonly minWidthInPx: number;
  readonly viewLineCounts: number[];
  private readonly drawn = new Map<number, Drawn>();
  private margin: HTMLElement | null = null;
  private signs = false;
  private shown = '';

  private readonly source: LineSourceLike;
  private readonly options: RenderOptionsLike;
  private readonly decorations: readonly Decoration[];
  private readonly dom: HTMLElement;

  constructor(source: LineSourceLike, options: RenderOptionsLike, decorations: readonly Decoration[], dom: HTMLElement) {
    this.source = source;
    this.options = options;
    this.decorations = decorations;
    this.dom = dom;
    this.viewLineCounts = source.lineTokens.map((_, i) => source.lineBreakData[i]?.breakOffsets.length ?? 1);
    this.rows = new ZoneRows(this.viewLineCounts);
    this.heightInLines = this.rows.rows;
    const segments = function* () {
      for (let i = 0; i < source.lineTokens.length; i++) {
        const text = source.lineTokens[i]!.getLineContent();
        const breaks = source.lineBreakData[i]?.breakOffsets;
        if (!breaks) yield text;
        else for (let b = 0, from = 0; b < breaks.length; from = breaks[b++]!) yield text.slice(from, breaks[b]);
      }
    };
    this.minWidthInPx = (widestColumns(segments(), options.tabSize) + options.scrollBeyondLastColumn) * options.typicalHalfwidthCharacterWidth;
    // As renderLines does: the zone takes the editor's font.
    applyFontInfo(dom, options.fontInfo);
    registerLazyZone(dom, this.rows);
  }

  /** The zone's margin, and whether it shows a − per row (Monaco's renderIndicators). */
  readonly attachMargin = (margin: HTMLElement, signs: boolean): void => {
    this.margin = margin;
    this.signs = signs;
  };

  readonly onDomNodeTop = (top: number): void => {
    const lh = this.options.lineHeight;
    // Monaco marks a zone it shows; one off screen gets a top a million px up.
    const near = this.dom.hasAttribute('monaco-visible-view-zone') ? chunksNear(this.rows, top, this.dom.ownerDocument.defaultView?.innerHeight ?? 1000, lh) : null;
    const key = near ? `${near[0]}-${near[1]}` : '';
    if (key === this.shown) return;
    this.shown = key;
    for (const [c, d] of this.drawn) {
      if (near && c >= near[0] && c <= near[1]) continue;
      if (this.holdsSelection(d.el)) continue;
      d.el.remove();
      d.margin?.remove();
      this.drawn.delete(c);
    }
    if (near) for (let c = near[0]; c <= near[1]; c++) if (!this.drawn.has(c)) this.draw(c);
  };

  /** Copying a selection reads the old text between its two ends (Monaco's copySelection): the
   * chunks holding them stay, however far they scroll; the lines between needn't be drawn. */
  private holdsSelection(el: HTMLElement): boolean {
    const sel = el.ownerDocument.getSelection();
    if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return false;
    const r = sel.getRangeAt(0);
    return el.contains(r.startContainer) || el.contains(r.endContainer);
  }

  private draw(c: number): void {
    const { start, end, firstRow, rows } = this.rows.chunk(c);
    const lh = this.options.lineHeight;
    const el = this.dom.ownerDocument.createElement('div');
    el.dataset.firstRow = String(firstRow);
    el.style.cssText = `position:absolute;top:${firstRow * lh}px;left:0;width:100%;height:${rows * lh}px;`;
    const s = this.source;
    const result = renderLines(new LineSource(s.lineTokens.slice(start, end), s.lineBreakData.slice(start, end), s.mightContainNonBasicASCII, s.mightContainRTL), this.options, this.decorationsOf(start, end), el);
    this.dom.appendChild(el);
    let margin: HTMLElement | null = null;
    if (this.margin && this.signs) {
      margin = this.dom.ownerDocument.createElement('div');
      margin.style.cssText = `position:absolute;top:${firstRow * lh}px;left:0;right:0;height:${rows * lh}px;`;
      let html = '';
      for (let i = 0; i < rows; i++) html += `<div class="${SIGN_CLASS}" style="position:absolute;top:${i * lh}px;width:${this.options.lineDecorationsWidth}px;height:${lh}px;right:0;"></div>`;
      margin.innerHTML = html;
      // Before Monaco's own lightbulb, as its signs are.
      this.margin.prepend(margin);
    }
    this.drawn.set(c, { el, margin, result, start });
  }

  /** The block's inner-change decorations on old lines `[start, end)`, renumbered from the chunk's
   * first line. renderLines marks every line `char-delete` (the whole line's text deleted) when it
   * gets none: a block that has some, with none in this chunk, gets one past the chunk's end. */
  private decorationsOf(start: number, end: number): Decoration[] {
    if (!this.decorations.length) return [];
    const out = this.decorations.filter((d) => d.range.endLineNumber > start && d.range.startLineNumber <= end).map((d) => new InlineDecoration(d.range.delta(-start), d.inlineClassName, d.type));
    if (!out.length) out.push(new InlineDecoration(new Range(end - start + 1, 1, end - start + 1, 1), this.decorations[0]!.inlineClassName, this.decorations[0]!.type));
    return out;
  }

  /** The old position (line within the block, column) at `node`/`offset`, as renderLines' result
   * gives it: Monaco's copy of a selection in the zone reads it. */
  getModelPositionAt(node: Node, offset: number): PositionLike | undefined {
    const chunk = (node instanceof Element ? node : node.parentElement)?.closest('.view-line')?.parentElement;
    for (const d of this.drawn.values()) if (d.el === chunk) return d.result.getModelPositionAt(node, offset)?.delta(d.start);
    return undefined;
  }

  /** The chunks drawn, for tests. */
  get drawnChunks(): number[] { return [...this.drawn.keys()].sort((a, b) => a - b); }
}

/** Monaco's call, rewritten: a block of LAZY_MIN_LINES or more is drawn lazily, others as before. */
export function renderDeletedLines(source: LineSourceLike, options: RenderOptionsLike, decorations: Decoration[], dom: HTMLElement): RenderLinesResultLike | LazyDeletedLines {
  return source.lineTokens.length >= LAZY_MIN_LINES ? new LazyDeletedLines(source, options, decorations, dom) : renderLines(source, options, decorations, dom);
}

/** Has Monaco's deleted-lines zones drawn by `renderDeletedLines`. False when the build didn't
 * rewrite Monaco (build/monacoDeletedZones.ts): its zones are then drawn whole, as before. */
export function installLazyDeletedZones(): boolean {
  const hook = (viewZonesModule as { deletedZoneHook?: { render?: unknown } }).deletedZoneHook;
  if (!hook) return false;
  hook.render = renderDeletedLines;
  return true;
}
