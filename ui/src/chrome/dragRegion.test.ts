// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { beginWindowDrag, DRAG_THRESHOLD, isDragTarget } from './dragRegion';
import type { WindowOps } from './windowChrome';

const ops = () => ({ startDragging: vi.fn(async () => {}) }) as unknown as WindowOps & { startDragging: ReturnType<typeof vi.fn> };

describe('which presses start a window move', () => {
  afterEach(() => { document.body.innerHTML = ''; });

  it('the region and the text in it do; its controls and what they hold do not', () => {
    document.body.innerHTML = `
      <div id="bar">
        <span id="title"><b id="bold">orbital</b></span>
        <button id="btn"><svg id="icon"></svg></button>
        <div role="tab" id="tab"><span id="label">x</span></div>
        <input id="field">
        <a href="#x" id="link">l</a>
        <div tabindex="0" id="focusable">f</div>
        <div tabindex="-1" id="unfocusable">u</div>
        <span data-no-window-drag id="marked">m</span>
      </div>
      <div id="outside"></div>`;
    const bar = document.getElementById('bar')!;
    const at = (id: string) => isDragTarget(document.getElementById(id), bar);
    expect(isDragTarget(bar, bar)).toBe(true);
    for (const id of ['title', 'bold', 'unfocusable']) expect(at(id), id).toBe(true);
    for (const id of ['btn', 'icon', 'tab', 'label', 'field', 'link', 'focusable', 'marked', 'outside']) expect(at(id), id).toBe(false);
    expect(isDragTarget(null, bar)).toBe(false);
  });
});

describe('a press on a drag region', () => {
  const move = (x: number, y: number, buttons = 1) => document.dispatchEvent(new MouseEvent('mousemove', { clientX: x, clientY: y, buttons }));

  it('moves the window once the pointer has gone the threshold with the button down, once', () => {
    const o = ops();
    beginWindowDrag({ button: 0, clientX: 100, clientY: 10 }, o);
    move(101, 11);
    move(100 + DRAG_THRESHOLD - 1, 10);
    expect(o.startDragging).not.toHaveBeenCalled();
    move(100 + DRAG_THRESHOLD, 10);
    move(140, 30);
    expect(o.startDragging).toHaveBeenCalledTimes(1);
  });

  it('a click (released before the threshold) never moves it, nor does a later move', () => {
    const o = ops();
    beginWindowDrag({ button: 0, clientX: 100, clientY: 10 }, o);
    move(101, 10);
    document.dispatchEvent(new MouseEvent('mouseup', { clientX: 101, clientY: 10 }));
    move(160, 40);
    expect(o.startDragging).not.toHaveBeenCalled();
  });

  it('a move with the button already up stops watching', () => {
    const o = ops();
    beginWindowDrag({ button: 0, clientX: 100, clientY: 10 }, o);
    move(160, 40, 0);
    move(170, 40, 1);
    expect(o.startDragging).not.toHaveBeenCalled();
  });

  it('only the left button drags', () => {
    const o = ops();
    beginWindowDrag({ button: 1, clientX: 100, clientY: 10 }, o);
    beginWindowDrag({ button: 2, clientX: 100, clientY: 10 }, o);
    move(160, 40, 7);
    expect(o.startDragging).not.toHaveBeenCalled();
  });
});
