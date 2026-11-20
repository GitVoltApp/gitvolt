// The rebase, pick, remote and forge handler groups, spread last into HANDLERS (index.ts). A group
// added here takes its methods out of PENDING (coverage.ts).
import type { HandlerTable } from '../transport';
import { forgeReadHandlers } from './forgeReads';
import { forgeWriteHandlers } from './forgeWrites';
import { pickHandlers } from './pick';
import { rebaseHandlers } from './rebase';
import { remoteHandlers } from './remotes';

export const PHASE4_HANDLERS = {
  ...rebaseHandlers,
  ...pickHandlers,
  ...remoteHandlers,
  ...forgeReadHandlers,
  ...forgeWriteHandlers,
} satisfies Partial<HandlerTable>;
