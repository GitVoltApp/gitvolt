// When the reveal panel opens and closes: pure. The caller feeds it the pointer's and keyboard's
// events and owns the one timeout `waiting` asks for (REVEAL_HOVER_MS).

/**
 * - `enterDotted`: the pointer is on the ⋮ row's dotted lane (or a folded merge's node or stub);
 * - `enterRow`: on the rest of that row;
 * - `enterPanel`: on the open panel;
 * - `leave`: on neither the row nor its panel;
 * - `click`: a click on the row (or Enter on it);
 * - `escape`, `clickOutside`: Esc, or a press anywhere but the row and its panel;
 * - `timer`: the hover wait ran out.
 */
export type HoverEvent = { kind: 'enterDotted' | 'enterRow' | 'enterPanel' | 'leave' | 'click' | 'escape' | 'clickOutside' | 'timer' };
export interface HoverState { open: boolean; pinned: boolean; waiting: boolean }

export const CLOSED: HoverState = { open: false, pinned: false, waiting: false };
const OPEN: HoverState = { open: true, pinned: false, waiting: false };

/** The next state. `hoverOpens`: Settings › Graph › "Open collapsed commits on hover"; off, only
 * a click (or Enter) opens the panel. A pinned panel stays until Esc or a click elsewhere. */
export function hoverStep(s: HoverState, e: HoverEvent, hoverOpens: boolean): HoverState {
  switch (e.kind) {
    case 'enterDotted':
      if (s.open) return s;
      return hoverOpens ? OPEN : s;
    case 'enterRow':
      if (s.open || s.waiting) return s;
      return hoverOpens ? { ...s, waiting: true } : s;
    case 'enterPanel':
      return s;
    case 'timer':
      return s.waiting ? OPEN : s;
    case 'leave':
      if (s.pinned) return s;
      return s.open || s.waiting ? CLOSED : s;
    case 'click':
      return { open: true, pinned: true, waiting: false };
    case 'escape':
    case 'clickOutside':
      return s.open || s.waiting ? CLOSED : s;
  }
}
