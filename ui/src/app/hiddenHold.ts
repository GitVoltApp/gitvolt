/** How long a hidden tab keeps what would take a moment to draw or open again: repo tabs are
 * often flipped back and forth to compare, and a quick return should find it as it was. */
export const HIDDEN_HOLD_MS = 30_000;

/**
 * From an effect's cleanup (<Activity> hides by cleaning up effects): runs `release` once `el`
 * has stayed hidden for `ms`, or at once when the cleanup was an unmount (`el` is out of the
 * document once the commit is done; a hidden one stays in it). Returns the cancel, for the effect
 * that runs again when it's shown. A one-shot timer, not a loop.
 */
export function releaseWhenHidden(el: Element | null, release: () => void, ms = HIDDEN_HOLD_MS): () => void {
  let done = false;
  const run = () => {
    if (done) return;
    cancel();
    release();
  };
  const hold = setTimeout(run, ms);
  const gone = setTimeout(() => {
    if (!el?.isConnected) run();
  }, 0);
  const cancel = () => {
    done = true;
    clearTimeout(hold);
    clearTimeout(gone);
  };
  return cancel;
}
