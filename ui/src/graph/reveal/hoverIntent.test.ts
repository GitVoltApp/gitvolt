import { describe, expect, it } from 'vitest';
import { CLOSED, hoverStep, type HoverEvent, type HoverState } from './hoverIntent';

const run = (events: HoverEvent['kind'][], hoverOpens = true, from: HoverState = CLOSED) =>
  events.reduce((s, kind) => hoverStep(s, { kind }, hoverOpens), from);

describe('hoverStep', () => {
  it('the dotted lane opens at once, the row after the timer', () => {
    expect(run(['enterDotted'])).toEqual({ open: true, pinned: false, waiting: false });
    expect(run(['enterRow'])).toEqual({ open: false, pinned: false, waiting: true });
    expect(run(['enterRow', 'enterRow'])).toEqual({ open: false, pinned: false, waiting: true });
    expect(run(['enterRow', 'timer'])).toEqual({ open: true, pinned: false, waiting: false });
    // Reaching the dotted lane while waiting opens at once.
    expect(run(['enterRow', 'enterDotted'])).toEqual({ open: true, pinned: false, waiting: false });
    // A stray timer does nothing.
    expect(run(['timer'])).toEqual(CLOSED);
  });

  it('leaving the row and the panel closes unless pinned', () => {
    expect(run(['enterDotted', 'enterPanel', 'enterRow', 'enterPanel'])).toEqual({ open: true, pinned: false, waiting: false });
    expect(run(['enterDotted', 'enterPanel', 'leave'])).toEqual(CLOSED);
    expect(run(['enterRow', 'leave'])).toEqual(CLOSED);
    expect(run(['enterRow', 'leave', 'timer'])).toEqual(CLOSED);
    expect(run(['click', 'leave'])).toEqual({ open: true, pinned: true, waiting: false });
  });

  it('a click pins until Escape or a click elsewhere', () => {
    expect(run(['enterDotted', 'click'])).toEqual({ open: true, pinned: true, waiting: false });
    expect(run(['enterRow', 'click'])).toEqual({ open: true, pinned: true, waiting: false });
    expect(run(['click', 'enterPanel', 'leave', 'enterRow'])).toEqual({ open: true, pinned: true, waiting: false });
    expect(run(['click', 'escape'])).toEqual(CLOSED);
    expect(run(['click', 'clickOutside'])).toEqual(CLOSED);
    expect(run(['enterDotted', 'escape'])).toEqual(CLOSED);
  });

  it('with hover-to-open off only a click opens', () => {
    expect(run(['enterDotted'], false)).toEqual(CLOSED);
    expect(run(['enterRow', 'timer'], false)).toEqual(CLOSED);
    expect(run(['enterRow', 'click', 'leave'], false)).toEqual({ open: true, pinned: true, waiting: false });
  });
});
