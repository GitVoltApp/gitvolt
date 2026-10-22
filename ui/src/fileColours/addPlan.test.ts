import { describe, expect, it } from 'vitest';
import { planAdd } from './addPlan';
import type { FileScope } from './store';

const sc = (name: string, patterns: string[]): FileScope => ({ id: name, name, tint: 'green', patterns, startCollapsed: false });
const scopes = [sc('Tests', ['**/tests/**']), sc('Snap', ['*.snap']), sc('Gen', ['**/gen/**'])];
const paths = ['a/tests/x.snap', 'b/y.snap', 'a/tests/gen/q.rs', 'c.rs'];

describe('planAdd', () => {
  it('says when a higher scope takes the files, and what moving up would recolour', () => {
    expect(planAdd(scopes, 2, '*.snap', paths)).toEqual({ matching: 2, shadowedBy: 0, recolours: 1 });
  });
  it('has no shadow when the target is the top', () => {
    expect(planAdd(scopes, 0, '*.snap', paths).shadowedBy).toBeNull();
  });
  it('plans a new scope at the end', () => {
    expect(planAdd(scopes, 3, 'c.rs', paths)).toEqual({ matching: 1, shadowedBy: null, recolours: 0 });
  });
});
