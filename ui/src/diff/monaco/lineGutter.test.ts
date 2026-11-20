// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { registerLazyZone, ZoneRows } from './deletedRows';
import { deletedLineAt, LineGutter, type LineGutterSpec } from './lineGutter';

vi.mock('./setup', () => ({
  monaco: {
    editor: {
      MouseTargetType: { GUTTER_GLYPH_MARGIN: 2, GUTTER_LINE_NUMBERS: 3, GUTTER_LINE_DECORATIONS: 4, GUTTER_VIEW_ZONE: 5, CONTENT_TEXT: 6, CONTENT_EMPTY: 7, CONTENT_VIEW_ZONE: 8, OVERLAY_WIDGET: 12 },
      EditorOption: { lineHeight: 75 },
    },
  },
}));

type Listener = (e: unknown) => void;
function fakeEditor() {
  const on: Record<string, Listener[]> = {};
  const sub = (name: string) => (cb: Listener) => { (on[name] ??= []).push(cb); return { dispose() {} }; };
  let widget: HTMLElement | null = null;
  return {
    on,
    widget: () => widget!,
    fire: (name: string, e: unknown) => on[name]?.forEach((l) => l(e)),
    addOverlayWidget: (w: { getDomNode(): HTMLElement }) => { widget = w.getDomNode(); },
    onMouseMove: sub('move'),
    onMouseLeave: sub('leave'),
    onDidScrollChange: sub('scroll'),
    onDidChangeModel: sub('model'),
    getLayoutInfo: () => ({ glyphMarginLeft: 0, glyphMarginWidth: 18 }),
    getOption: () => 19,
    getTopForLineNumber: (n: number) => (n - 1) * 19,
    getScrollTop: () => 0,
    getDomNode: () => null,
  };
}

const move = (type: number, lineNumber: number) => ({ target: { type, position: { lineNumber } }, event: { posy: 0 } });

describe("the gutter's line button", () => {
  function setup(spec: Partial<LineGutterSpec> = {}) {
    const original = fakeEditor();
    const modified = fakeEditor();
    const gutter = new LineGutter({ getOriginalEditor: () => original, getModifiedEditor: () => modified, getLineChanges: () => [] } as never);
    const onLine = vi.fn();
    gutter.set({ old: new Set([5]), new: new Set([5, 9]), staged: false, disabled: null, onLine, ...spec });
    return { gutter, original, modified, onLine };
  }

  it('shows + on a hovered changed line, in its glyph cell, and stages that line; nothing on other lines', () => {
    const { modified, onLine } = setup();
    const btn = modified.widget();
    expect(btn.hidden).toBe(true);
    modified.fire('move', move(6, 9));
    expect(btn.hidden).toBe(false);
    expect(btn.style.top).toBe(`${8 * 19}px`);
    expect(btn.getAttribute('aria-label')).toBe('Stage this line');
    expect(btn.textContent).toBe('+');
    btn.click();
    expect(onLine).toHaveBeenCalledWith('modified', 9);
    // Over the button itself, it stays; on an unchanged line, it goes.
    modified.fire('move', { target: { type: 12 }, event: { posy: 0 } });
    expect(btn.hidden).toBe(false);
    modified.fire('move', move(6, 7));
    expect(btn.hidden).toBe(true);
  });

  it("the original side's - lines get it too; a scroll hides it", () => {
    const { original, onLine } = setup();
    original.fire('move', move(3, 5));
    expect(original.widget().hidden).toBe(false);
    original.widget().click();
    expect(onLine).toHaveBeenCalledWith('original', 5);
    original.fire('scroll', {});
    expect(original.widget().hidden).toBe(true);
  });

  it('staged: a red − that unstages; a reason disables it, as its tooltip', () => {
    const { modified, onLine } = setup({ staged: true, disabled: 'Save first' });
    modified.fire('move', move(6, 5));
    const btn = modified.widget();
    expect(btn.classList.contains('unstage')).toBe(true);
    expect(btn.getAttribute('aria-label')).toBe('Unstage this line');
    expect(btn.title).toBe('Save first');
    expect(btn.getAttribute('aria-disabled')).toBe('true');
    btn.click();
    expect(onLine).not.toHaveBeenCalled();
  });

  it('cleared, nothing shows', () => {
    const { gutter, modified } = setup();
    modified.fire('move', move(6, 9));
    gutter.set(null);
    expect(modified.widget().hidden).toBe(true);
    modified.fire('move', move(6, 9));
    expect(modified.widget().hidden).toBe(true);
  });
});

describe('the old line under the pointer in a deleted-lines zone', () => {
  it('in a large block drawn a chunk at a time: from its rows, with none of its lines drawn', () => {
    // Old lines 10-609 deleted, after new line 4; the zone 100 px down the editor, its old line 11
    // wrapping onto two rows.
    const root = document.createElement('div');
    root.innerHTML = '<div class="view-zones"><div class="line-delete" monaco-view-zone="z1"></div></div>';
    const zone = root.querySelector<HTMLElement>('.line-delete')!;
    vi.spyOn(zone, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 100, 400, 601 * 19));
    registerLazyZone(zone, new ZoneRows(Array.from({ length: 600 }, (_, i) => (i === 1 ? 2 : 1))));
    const ed = Object.assign(fakeEditor(), { getDomNode: () => root });
    const diff = {
      getLineChanges: () => [{ originalStartLineNumber: 10, originalEndLineNumber: 609, modifiedStartLineNumber: 4, modifiedEndLineNumber: 0 }],
      getOriginalEditor: () => ({ getModel: () => ({ getLineContent: () => 'old' }) }),
    };
    const at = (row: number) => deletedLineAt(diff as never, ed as never, 'z1', 4, 100 + row * 19 + 5);
    expect(at(0)).toEqual({ line: 10, top: 100 });
    // Both rows of line 11 give it, at its first row's top.
    expect(at(1)).toEqual({ line: 11, top: 119 });
    expect(at(2)).toEqual({ line: 11, top: 119 });
    expect(at(3)).toEqual({ line: 12, top: 100 + 3 * 19 });
    expect(at(600)).toEqual({ line: 609, top: 100 + 600 * 19 });
  });
});
