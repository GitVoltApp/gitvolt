import type { Page } from '@playwright/test';

/**
 * Branch `name`'s chip in the commit graph: the chip, not its name. A hovered chip floats its
 * full-name copy (`.ref-label-full`, a child of the chip) over its name, so a click aimed at the
 * name lands on the copy, which Playwright counts as intercepted and retries until the test times
 * out. WebKit draws the copy between the pointer's arrival and the press once the app is idle;
 * Chromium dispatches the press first. The copy is inside the chip, so a click on the chip passes.
 */
export const graphChip = (page: Page, name: string) =>
  page.getByRole('grid', { name: 'Commit graph' }).locator('.ref-label', { has: page.getByText(name, { exact: true }) }).first();
