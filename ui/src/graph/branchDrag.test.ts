// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphPayload } from '../api/gen/GraphPayload';
import { labelOf, startBranchDrag, useBranchDrag } from './branchDrag';

const chip = (ref: string | null) => {
  const el = document.createElement('span');
  if (ref) el.setAttribute('data-drag-branch', ref);
  document.body.append(el);
  return el;
};
let chipA: HTMLElement;
let chipB: HTMLElement;
let scroller: HTMLElement;
let drop: ReturnType<typeof vi.fn<(from: string, onto: string, x: number, y: number) => void>>;

/** happy-dom does no layout: the element under the pointer is the one `elementFromPoint` is told. */
const pointAt = (el: Element | null) => { document.elementFromPoint = () => el; };
const press = (x: number, y: number, mods: Partial<{ button: number; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) =>
  ({ button: 0, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, clientX: x, clientY: y, ...mods });
const pointer = (type: string, x: number, y: number) => window.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y }));
const move = (x: number, y: number) => pointer('pointermove', x, y);
const up = (x: number, y: number) => pointer('pointerup', x, y);
const esc = () => {
  const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
  window.dispatchEvent(e);
  return e;
};
/** A click on the page: whether it got through, or a drag's release swallowed it. */
const click = () => {
  let seen = false;
  const see = () => { seen = true; };
  window.addEventListener('click', see);
  document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  window.removeEventListener('click', see);
  return seen ? 'delivered' : 'swallowed';
};
const start = () => startBranchDrag(press(10, 100), 'refs/heads/a', scroller, drop);

beforeEach(() => {
  chipA = chip('refs/heads/a');
  chipB = chip('refs/heads/b');
  scroller = document.createElement('div');
  drop = vi.fn();
});
afterEach(() => {
  esc();
  up(0, 0);
  click();
  document.body.replaceChildren();
  useBranchDrag.setState({ drag: null });
});

describe('dragging a branch chip', () => {
  it('a press that travels under the threshold is still a click', () => {
    start();
    pointAt(chipB);
    move(12, 101);
    up(12, 101);
    expect(useBranchDrag.getState().drag).toBeNull();
    expect(drop).not.toHaveBeenCalled();
    expect(click()).toBe('delivered');
  });

  it('past the threshold it is a drag: the store follows the pointer and the branch under it', () => {
    start();
    pointAt(chipB);
    move(10, 90);
    expect(useBranchDrag.getState().drag).toEqual({ from: 'refs/heads/a', over: 'refs/heads/b', x: 10, y: 90 });
    pointAt(chipA);
    move(10, 80);
    expect(useBranchDrag.getState().drag).toEqual({ from: 'refs/heads/a', over: null, x: 10, y: 80 });
  });

  it('a release on another branch drops A on B once, at the release point, and swallows its click', () => {
    start();
    pointAt(chipB);
    move(10, 90);
    up(10, 90);
    expect(drop).toHaveBeenCalledExactlyOnceWith('refs/heads/a', 'refs/heads/b', 10, 90);
    expect(useBranchDrag.getState().drag).toBeNull();
    expect(click()).toBe('swallowed');
  });

  it("a release on a branch's expanded copy drops on that branch", () => {
    const copy = document.createElement('span');
    chipB.append(copy);
    start();
    pointAt(copy);
    move(10, 90);
    up(10, 90);
    expect(drop).toHaveBeenCalledExactlyOnceWith('refs/heads/a', 'refs/heads/b', 10, 90);
  });

  it('a release on itself, on no chip, on a non-branch element or on a tag row of a branch\'s hover stack drops nothing', () => {
    const tagRow = document.createElement('span');
    tagRow.className = 'ref-stack-row';
    chipB.append(tagRow);
    for (const target of [chipA, null, chip(null), tagRow]) {
      start();
      pointAt(target);
      move(10, 90);
      up(10, 90);
      expect(useBranchDrag.getState().drag).toBeNull();
      expect(click()).toBe('swallowed');
    }
    expect(drop).not.toHaveBeenCalled();
  });

  it('Esc ends the drag, and the coming release clicks nothing', () => {
    start();
    pointAt(chipB);
    move(10, 90);
    expect(esc().defaultPrevented).toBe(true);
    expect(useBranchDrag.getState().drag).toBeNull();
    up(10, 90);
    expect(drop).not.toHaveBeenCalled();
    expect(click()).toBe('swallowed');
    expect(click()).toBe('delivered');
  });

  it('Esc before the threshold is not taken', () => {
    start();
    move(11, 101);
    expect(esc().defaultPrevented).toBe(false);
  });

  it('blur and pointercancel end the drag', () => {
    for (const end of [() => window.dispatchEvent(new Event('blur')), () => pointer('pointercancel', 10, 90)]) {
      start();
      pointAt(chipB);
      move(10, 90);
      end();
      expect(useBranchDrag.getState().drag).toBeNull();
      up(10, 90);
      expect(drop).not.toHaveBeenCalled();
    }
  });

  it('a right, Ctrl, Cmd, Shift or Alt press starts nothing', () => {
    for (const mods of [{ button: 2 }, { ctrlKey: true }, { metaKey: true }, { shiftKey: true }, { altKey: true }]) {
      startBranchDrag(press(10, 100, mods), 'refs/heads/a', scroller, drop);
      pointAt(chipB);
      move(10, 50);
      expect(useBranchDrag.getState().drag, JSON.stringify(mods)).toBeNull();
    }
  });

  it('a new press ends a drag left over', () => {
    start();
    move(10, 90);
    startBranchDrag(press(50, 100), 'refs/heads/b', scroller, drop);
    pointAt(chipA);
    move(50, 90);
    up(50, 90);
    expect(drop).toHaveBeenCalledExactlyOnceWith('refs/heads/b', 'refs/heads/a', 50, 90);
  });

  it('ends with its listeners removed', () => {
    const removed = vi.spyOn(window, 'removeEventListener');
    start();
    move(10, 90);
    up(10, 90);
    expect(removed.mock.calls.map((c) => c[0])).toEqual(expect.arrayContaining(['pointermove', 'pointerup', 'pointercancel', 'blur']));
    removed.mockRestore();
  });

  describe('near the edge the graph scrolls', () => {
    let frames: FrameRequestCallback[];
    const flushFrame = () => frames.splice(0).forEach((f) => f(0));
    beforeEach(() => {
      frames = [];
      vi.spyOn(window, 'requestAnimationFrame').mockImplementation((f) => { frames.push(f); return frames.length; });
      vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => {});
      scroller.getBoundingClientRect = () => ({ top: 100, bottom: 500 }) as DOMRect;
      scroller.scrollTop = 200;
    });
    afterEach(() => vi.restoreAllMocks());

    it('the scroller steps near an edge and re-hit-tests the target', () => {
      start();
      pointAt(null);
      move(10, 110);
      flushFrame();
      expect(scroller.scrollTop).toBe(200 - 11);
      pointAt(chipB);
      flushFrame();
      expect(scroller.scrollTop).toBe(200 - 22);
      expect(useBranchDrag.getState().drag).toEqual({ from: 'refs/heads/a', over: 'refs/heads/b', x: 10, y: 110 });
      move(10, 495);
      flushFrame();
      expect(scroller.scrollTop).toBe(200 - 22 + 12);
    });

    it('the loop stops once the graph can scroll no further that way', () => {
      let top = 4;
      Object.defineProperty(scroller, 'scrollTop', { configurable: true, get: () => top, set: (v: number) => { top = Math.max(0, v); } });
      start();
      move(10, 110);
      flushFrame();
      expect(top).toBe(0);
      expect(frames).toHaveLength(1);
      flushFrame();
      expect(frames).toHaveLength(0);
    });

    it('no frame is asked for away from the edges, and the loop stops on leaving the zone or ending the drag', () => {
      start();
      move(10, 300);
      expect(frames).toHaveLength(0);
      move(10, 110);
      move(10, 112);
      expect(frames).toHaveLength(1);
      move(10, 300);
      flushFrame();
      expect(frames).toHaveLength(0);
      expect(scroller.scrollTop).toBe(200);
      move(10, 110);
      esc();
      flushFrame();
      expect(frames).toHaveLength(0);
      expect(scroller.scrollTop).toBe(200);
    });
  });

  it("labelOf finds a local ref's label and row, and nothing for an unknown ref", () => {
    const row = (id: string) => ({ id, kind: 'commit', lane: 0, color: 0, segments: [], summary: id, bodyFirstLine: '', authorName: '', authorEmail: '', authorTime: 0, committerTime: 0, parents: [], mrRefs: [], wip: null }) as const;
    const label = (name: string, r: number, local: string | null = `refs/heads/${name}`) => ({ row: r, name, local, remotes: [], tag: false, isHead: false, worktree: null, checkedOut: null });
    const graph = { rows: [row('m'), row('x')], labels: [label('main', 0), label('v1', 0, null), label('feature/x', 1)] } as unknown as GraphPayload;
    expect(labelOf(graph, 'refs/heads/feature/x')).toEqual({ row: graph.rows[1], label: graph.labels[2] });
    expect(labelOf(graph, 'refs/heads/gone')).toBeNull();
  });
});
