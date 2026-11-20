/**
 * A blur that only means the WINDOW lost focus, not that the user moved on. Under
 * GNOME, mutter answers every mouse press with WM_TAKE_FOCUS and GitVolt's CEF host refocuses
 * its browser (vendor/tauri-runtime-cef platform/linux/focus.rs), so the page sees a window blur
 * then a focus a few ms later — and the focused element gets a `blur` in between. An input that
 * commits on blur would end editing on a click inside itself. `document.hasFocus()` is false
 * during exactly those blurs; a real move to another element of the page keeps it true.
 */
export function isWindowBlur(): boolean {
  return !document.hasFocus();
}

/**
 * Called from a window-blur `onBlur`: once the window has focus again, puts the caret back in
 * `el` (the browser doesn't always restore it after the WM's bounce), unless `still()` says
 * editing ended meanwhile.
 */
export function refocusWhenWindowReturns(el: HTMLElement | null, still: () => boolean = () => true): void {
  const back = () => {
    window.removeEventListener('focus', back);
    if (el && el.isConnected && still() && document.activeElement !== el) el.focus({ preventScroll: true });
  };
  window.addEventListener('focus', back);
}

/** How long the window must stay without focus before `data-window-blurred` is set: longer than the
 * WM's bounce above (a few ms), so a click doesn't pause and resume what it styles. */
export const BLURRED_AFTER_MS = 1_000;

/**
 * Sets `data-window-blurred` on the root while the window is without focus, for styles that pause
 * what only decorates there: a forge pipeline's spinner (forge/mr.css) runs on the forge's work,
 * not GitVolt's, and would keep the renderer drawing for as long as the window stays in the
 * background. Returns the uninstall.
 */
export function installWindowBlurredMark(root: HTMLElement = document.documentElement): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const settle = () => {
    timer = null;
    root.toggleAttribute('data-window-blurred', !document.hasFocus());
  };
  const onBlur = () => {
    if (timer === null) timer = setTimeout(settle, BLURRED_AFTER_MS);
  };
  const onFocus = () => {
    if (timer !== null) clearTimeout(timer);
    settle();
  };
  window.addEventListener('blur', onBlur);
  window.addEventListener('focus', onFocus);
  if (!document.hasFocus()) onBlur();
  return () => {
    window.removeEventListener('blur', onBlur);
    window.removeEventListener('focus', onFocus);
    if (timer !== null) clearTimeout(timer);
    root.removeAttribute('data-window-blurred');
  };
}
