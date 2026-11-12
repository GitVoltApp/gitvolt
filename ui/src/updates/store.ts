import { create } from 'zustand';
import { api, errorMessage, onEvent } from '../api/client';
import type { InstallOutcome } from '../api/gen/InstallOutcome';
import type { UpdateRelease } from '../api/gen/UpdateRelease';
import type { UpdateState } from '../api/gen/UpdateState';
import { flushSaves } from '../app/state';
import { confirmDiscardUnsaved, unsavedKeys } from '../diff/workingCopy';

/**
 * The update the core found (core `updates.rs`): its state as `updateChanged` events report it,
 * the dialog, and the last manual check's or install's answer. The webview never talks to
 * GitHub; every step is a request to the core.
 */
interface UpdatesStore {
  state: UpdateState;
  dialogOpen: boolean;
  /** Check for updates (About, the Help menu) failed: why. Cleared by the next check. */
  checkError: string | null;
  /** What the last Install said when it couldn't install by itself (the command to run). */
  outcome: InstallOutcome | null;
  /** A request that failed outside a check (download, install, restart). */
  error: string | null;
  /** Install and restart is under way: a second press does nothing. */
  running: boolean;
  setState(state: UpdateState): void;
  /** Starts listening and reads the current state, once. */
  listen(): void;
  check(): Promise<void>;
  /**
   * The dialog's one action: downloads the update (unless it already is), installs it, and
   * restarts GitVolt, each step only on the last one's success. Unsaved edits ask first, before
   * anything starts, and again before the restart only for edits made since. It runs here, not
   * in the dialog, so closing the dialog doesn't stop it.
   */
  installAndRestart(): Promise<void>;
  cancel(): Promise<void>;
  openDialog(): void;
  closeDialog(): void;
}

/** An update a download can start from: offered, or offered again after a failure. */
const downloadable = (s: UpdateState) => s.state === 'available' || (s.state === 'failed' && s.release !== null);

let listening = false;

export const useUpdates = create<UpdatesStore>((set, get) => {
  const failed = (e: unknown) => set({ error: errorMessage(e) });
  return {
    state: { state: 'idle' },
    dialogOpen: false,
    checkError: null,
    outcome: null,
    error: null,
    running: false,
    setState: (state) => set((s) => ({ state, outcome: state.state === 'ready' || state.state === 'installing' ? s.outcome : null })),
    listen() {
      if (listening) return;
      listening = true;
      onEvent((ev) => { if (ev.type === 'updateChanged') get().setState(ev.state); });
      Promise.resolve().then(() => api.updateStatus()).then((s) => get().setState(s), (e: unknown) => console.warn('[gitvolt] update status', errorMessage(e)));
    },
    async check() {
      set({ checkError: null });
      try { get().setState(await api.updateCheck()); } catch (e) { set({ checkError: errorMessage(e) }); }
    },
    async installAndRestart() {
      if (get().running) return;
      set({ running: true, error: null, outcome: null });
      try {
        // Asked now, not once it's installed. Cancelled: nothing starts.
        if (!(await confirmDiscardUnsaved())) return;
        const agreed = unsavedKeys();
        if (get().state.state !== 'installed') {
          if (downloadable(get().state)) {
            const started = await api.updateDownload();
            // Its events may have moved it on already (a small, fast download).
            if (downloadable(get().state)) get().setState(started);
          }
          // Cancelled (offered again) or failed: the dialog and the pill say so.
          if ((await downloadEnded()).state !== 'ready') return;
          await flushSaves();
          const outcome = await api.updateInstall();
          set({ outcome });
          // Windows: the installer took over and GitVolt is quitting. Manual: the dialog shows
          // the command to run instead.
          if (outcome.outcome !== 'installed') return;
        }
        // Edits made since the answer ask again; kept, the update waits on Restart GitVolt.
        if ([...unsavedKeys()].some((k) => !agreed.has(k)) && !(await confirmDiscardUnsaved())) return;
        // Settings changed meanwhile are saved too.
        await flushSaves();
        await api.updateRestart();
      } catch (e) { failed(e); } finally { set({ running: false }); }
    },
    async cancel() {
      try { get().setState(await api.updateCancel()); } catch (e) { failed(e); }
    },
    openDialog: () => set({ dialogOpen: true }),
    closeDialog: () => set({ dialogOpen: false }),
  };
});

/** The state once the download under way, if any, has ended (ready, failed, or cancelled). */
function downloadEnded(): Promise<UpdateState> {
  const now = useUpdates.getState().state;
  if (now.state !== 'downloading') return Promise.resolve(now);
  return new Promise((resolve) => {
    const off = useUpdates.subscribe(({ state }) => {
      if (state.state === 'downloading') return;
      off();
      resolve(state);
    });
  });
}

/**
 * Check for updates from About (its button, Help › Check for updates): an update it finds, or one
 * already known, opens the update dialog right away, with `closeAbout` in the same update so About
 * closes under it.
 */
export async function checkAndShow(closeAbout: () => void): Promise<void> {
  await useUpdates.getState().check();
  const { state, checkError } = useUpdates.getState();
  if (checkError || !releaseOf(state)) return;
  closeAbout();
  useUpdates.getState().openDialog();
}

/** The release the pill and the dialog show, if any. A check that failed has none. */
export function releaseOf(s: UpdateState): UpdateRelease | null {
  return 'release' in s ? s.release : null;
}

/** 0–100, for the download's bar. */
export function percentOf(s: UpdateState): number {
  if (s.state !== 'downloading' || s.total <= 0) return 0;
  return Math.min(100, Math.floor((s.received * 100) / s.total));
}

/** `98.2 MB`, `640 KB`. */
export function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Tests only: forget the subscription. */
export function resetUpdatesForTest(): void {
  listening = false;
  useUpdates.setState({ state: { state: 'idle' }, dialogOpen: false, checkError: null, outcome: null, error: null, running: false });
}
