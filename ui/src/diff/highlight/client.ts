// The UI side of the highlight worker: starts it once (with every editor theme), and routes its
// replies. The worker is kept for the session once started: it costs nothing while idle (no timer
// runs in it), and a new one would load the Oniguruma WASM and the grammars again. Without a
// Worker (tests), or if it can't start, the same engine runs on the main thread.
import { editorThemeRegistrations } from '../../theme/editorThemes';
import { memoizeUntilRejected } from '../monaco/memo';
import type { CodeTokens, FromWorker, ThemeInfo, ToWorker } from './protocol';

interface Port {
  post(msg: ToWorker, transfer?: Transferable[]): void;
  onmessage: (msg: FromWorker) => void;
  onerror: () => void;
  close(): void;
}

export type DocReply = Extract<FromWorker, { kind: 'tokens' | 'done' }>;

export interface Client {
  readonly themes: ThemeInfo[];
  post(msg: ToWorker): void;
  /** Loads a grammar in the worker: every language loaded so far, or `null` if it failed. */
  loadLanguage(lang: string): Promise<string[] | null>;
  code(code: string, lang: string, theme: string): { result: Promise<CodeTokens | null>; cancel(): void };
  /** Where a text's tokens go (modelTokens.ts), until `undefined`. */
  route(doc: number, to: ((msg: DocReply) => void) | undefined): void;
}

function workerPort(): Port | null {
  if (typeof Worker !== 'function') return null;
  let w: Worker;
  try {
    w = new Worker(new URL('./highlight.worker.ts', import.meta.url), { type: 'module' });
  } catch {
    return null;
  }
  const port: Port = { post: (m, t) => w.postMessage(m, t ?? []), onmessage: () => {}, onerror: () => {}, close: () => w.terminate() };
  w.onmessage = (e: MessageEvent<FromWorker>) => port.onmessage(e.data);
  w.onerror = (e) => { e.preventDefault(); port.onerror(); };
  return port;
}

async function mainThreadPort(): Promise<Port> {
  const [{ Engine }, { shikiGrammars }] = await Promise.all([import('./engine'), import('./shikiGrammars')]);
  const port: Port = { post: (m) => engine.handle(m), onmessage: () => {}, onerror: () => {}, close: () => {} };
  const engine = new Engine(shikiGrammars(), (m) => queueMicrotask(() => port.onmessage(m)));
  return port;
}

/** Sends `init` and waits for the highlighter: its themes, or the reason it failed. */
function handshake(port: Port, themes: ToWorker & { kind: 'init' }): Promise<ThemeInfo[]> {
  return new Promise((resolve, reject) => {
    port.onerror = () => reject(new Error('the highlight worker failed to start'));
    port.onmessage = (m) => {
      if (m.kind === 'ready') resolve(m.themes);
      else if (m.kind === 'failed') reject(new Error(m.message));
    };
    port.post(themes);
  });
}

async function connect(): Promise<Client> {
  const init = { kind: 'init', themes: await Promise.all(editorThemeRegistrations()) } as const;
  let port = workerPort();
  let themes: ThemeInfo[] | null = null;
  if (port) {
    themes = await handshake(port, init).catch(() => null);
    if (!themes) port.close();
  }
  if (!themes) {
    port = await mainThreadPort();
    themes = await handshake(port, init);
  }
  const p = port!;
  let nextId = 0;
  const langs = new Map<number, (loaded: string[] | null) => void>();
  const codes = new Map<number, (t: CodeTokens | null) => void>();
  const docs = new Map<number, (msg: DocReply) => void>();
  p.onerror = () => console.warn('[highlight] the worker stopped: new text stays plain');
  p.onmessage = (m) => {
    if (m.kind === 'lang') { langs.get(m.id)?.(m.loaded); langs.delete(m.id); }
    else if (m.kind === 'code') { codes.get(m.id)?.(m.tokens); codes.delete(m.id); }
    else if (m.kind === 'tokens' || m.kind === 'done') docs.get(m.doc)?.(m);
  };
  return {
    themes,
    post: (m) => p.post(m),
    loadLanguage(lang) {
      const id = ++nextId;
      return new Promise((resolve) => {
        langs.set(id, resolve);
        p.post({ kind: 'lang', id, lang });
      });
    },
    code(code, lang, theme) {
      const id = ++nextId;
      const result = new Promise<CodeTokens | null>((resolve) => codes.set(id, resolve));
      p.post({ kind: 'code', id, code, lang, theme });
      return {
        result,
        cancel: () => {
          if (!codes.delete(id)) return;
          p.post({ kind: 'cancel', id });
        },
      };
    },
    route(doc, to) {
      if (to) docs.set(doc, to);
      else docs.delete(doc);
    },
  };
}

/** The app's one highlighter (a failed start is tried again on the next call). */
export const highlightClient: () => Promise<Client> = memoizeUntilRejected(connect);
