import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanClosedTab, onRepoClosed, onTabClosed } from './closedTabs';

describe('closed tab cleaners', () => {
  const offs: Array<() => void> = [];
  afterEach(() => { for (const off of offs.splice(0)) off(); });

  it("a closing tab's cleaners run; its repository's run only when no other tab shows it", () => {
    const tab = vi.fn();
    const repo = vi.fn();
    offs.push(onTabClosed(tab), onRepoClosed(repo));
    const tabs = { a: { repo: { id: 1 } }, b: { repo: { id: 1 } }, c: { repo: { id: 2 } }, d: { repo: null } };
    cleanClosedTab('a', tabs);
    expect(tab).toHaveBeenCalledWith('a');
    expect(repo).not.toHaveBeenCalled(); // b still shows repository 1
    const { a: _a, ...rest } = tabs;
    cleanClosedTab('b', rest);
    expect(repo).toHaveBeenCalledExactlyOnceWith(1);
    cleanClosedTab('d', rest); // never opened: its tab's state only
    cleanClosedTab('e', rest); // no runtime at all
    expect(tab.mock.calls.map((c) => c[0])).toEqual(['a', 'b', 'd', 'e']);
    expect(repo).toHaveBeenCalledOnce();
  });

  it('an unregistered cleaner no longer runs', () => {
    const repo = vi.fn();
    onRepoClosed(repo)();
    cleanClosedTab('c', { c: { repo: { id: 2 } } });
    expect(repo).not.toHaveBeenCalled();
  });
});
