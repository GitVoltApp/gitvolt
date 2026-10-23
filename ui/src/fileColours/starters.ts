import type { FileScope } from './store';

export const STARTER_SCOPES: readonly Omit<FileScope, 'id'>[] = [
  {
    name: 'Tests',
    tint: 'green',
    patterns: ['**/tests/**', '**/test/**', '**/__tests__/**', '*.test.*', '*.spec.*', '*_test.*', '*.snap'],
    startCollapsed: false,
  },
  {
    name: 'Generated',
    tint: 'gray',
    patterns: ['**/generated/**', '*.generated.*', '*.pb.*', '*.min.js', '*.lock', 'package-lock.json'],
    startCollapsed: true,
  },
  { name: 'Docs', tint: 'blue', patterns: ['docs/**', '*.md', '*.rst', '*.adoc'], startCollapsed: false },
];
