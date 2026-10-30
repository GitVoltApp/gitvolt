import { fireEvent } from '@testing-library/react';

const mouse = { pointerType: 'mouse', pointerId: 1 };

/** Tests only: a mouse press at (x, y) on `el` (`button` 2: a right press). */
export const mouseDown = (el: Element, x = 10, y = 10, button = 0) => fireEvent.pointerDown(el, { ...mouse, button, buttons: button === 2 ? 2 : 1, clientX: x, clientY: y });

/** Tests only: the press's move to (x, y) over `el` and its release there, then the click a left
 * button's release makes on what holds both the press and the release (`clickOn`). */
export function mouseUp(el: Element, x: number, y: number, clickOn: Element | null = null, button = 0): void {
  fireEvent.pointerMove(el, { ...mouse, clientX: x, clientY: y });
  fireEvent.pointerUp(el, { ...mouse, button, clientX: x, clientY: y });
  if (clickOn && button === 0) fireEvent.click(clickOn, { detail: 1, clientX: x, clientY: y });
}

/** Tests only: a mouse release with no move first. */
export const mouseRelease = (el: Element, x = 10, y = 10, button = 0) => fireEvent.pointerUp(el, { ...mouse, button, clientX: x, clientY: y });
