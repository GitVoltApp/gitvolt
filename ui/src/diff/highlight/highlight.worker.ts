// The highlight worker: Shiki's grammars tokenize here, off the UI thread (engine.ts). It sits idle,
// with no timer, while nothing is pending; the app keeps the one it started (client.ts).
import { Engine } from './engine';
import type { ToWorker } from './protocol';
import { shikiGrammars } from './shikiGrammars';

const engine = new Engine(shikiGrammars(), (msg, transfer) => self.postMessage(msg, { transfer }));
self.onmessage = (e: MessageEvent<ToWorker>) => engine.handle(e.data);
