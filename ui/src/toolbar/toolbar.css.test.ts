import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// The DOM tests don't load CSS: this pins the split button's rules (the mockup toolbar-split.html).
const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'toolbar.css'), 'utf8');
const rule = (selector: string) => css.match(new RegExp(`(?:^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`))?.[1] ?? '';

describe('toolbar split buttons', () => {
  it('separate actions sit 12px apart; a split button has no gap between its halves', () => {
    expect(rule('.tb-center, .tb-end')).toMatch(/gap:\s*12px/);
    expect(rule('.tb-split')).not.toMatch(/\bgap\s*:/);
  });

  it('the ▾ is 14px wide and as tall as its button; the main part keeps its 44×38 minimum', () => {
    expect(rule('.tb-caret')).toMatch(/min-width:\s*14px;\s*width:\s*14px/);
    expect(rule('.tb-caret')).not.toMatch(/height/);
    expect(rule('.tb-btn')).toMatch(/min-width:\s*44px;\s*height:\s*38px/);
  });

  it('the pair is one rounded outline: the inner corners are square', () => {
    expect(rule('.tb-split > .tb-split-main, .tb-split > .tb-picker')).toMatch(/border-radius:\s*4px 0 0 4px/);
    expect(rule('.tb-split > .tb-caret')).toMatch(/border-radius:\s*0 4px 4px 0/);
  });

  it('hovered, the pair lights: the other half faintly (the hovered one has the full hover)', () => {
    expect(css).toMatch(/\.tb-split:hover > :is\(\.tb-btn, \.tb-picker\):not\(:hover\)[^{]*\{\s*background:\s*color-mix\(in srgb, var\(--hover-row\) 50%, transparent\)/);
    expect(rule('.tb-btn:hover:not(:disabled):not([aria-disabled="true"])')).toMatch(/background:\s*var\(--hover-row\)/);
  });

  it('each half keeps its own focus ring, inset so it never overlaps the other half', () => {
    expect(rule('.tb-split > .tb-btn:focus-visible')).toMatch(/outline-offset:\s*-1px/);
  });

  it('a hidden button keeps its place', () => {
    expect(rule('.tb-hidden')).toMatch(/visibility:\s*hidden/);
  });
});
