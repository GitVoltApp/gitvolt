import { createContext } from 'react';
import type { MarkdownContext } from './types';

/** Inside a removed part of a rendered diff, the context links, images and references resolve
 * against: the old side's commit and path. `null` everywhere else. */
export const MdContextOverride = createContext<MarkdownContext | null>(null);
