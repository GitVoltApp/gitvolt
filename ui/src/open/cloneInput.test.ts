import { describe, expect, it } from 'vitest';
import { cloneDestProblem, cloneUrlProblem } from './cloneInput';

describe('cloneUrlProblem', () => {
  it('accepts the supported forms', () => {
    for (const u of ['https://h/a/b.git', 'http://h/a', 'ssh://git@h:2222/a/b', 'git://h/a.git', 'file:///tmp/o.git', 'git@github.com:o/r.git', '/tmp/x/origin.git', '']) {
      expect(cloneUrlProblem(u), u).toBeNull();
    }
  });
  it('takes a Windows path only on Windows', () => {
    for (const u of ['C:\\repos\\origin.git', 'D:/repos/origin', '\\\\server\\share\\origin.git']) {
      expect(cloneUrlProblem(u, 'windows'), u).toBeNull();
      expect(cloneUrlProblem(u, 'linux'), u).not.toBeNull();
    }
  });
  it('rejects anything else', () => {
    for (const u of ['hello', 'ext::sh -c x', '-oProxyCommand=x', 'relative/path', 'https://', 'ftp://h/a', '/', 'C:repos\\origin']) {
      expect(cloneUrlProblem(u), u).not.toBeNull();
    }
  });
});

describe('cloneDestProblem', () => {
  it('requires an absolute path', () => {
    expect(cloneDestProblem('/home/u/r')).toBeNull();
    expect(cloneDestProblem('')).toBeNull();
    expect(cloneDestProblem('rel/dir')).not.toBeNull();
  });
  it('accepts Windows absolute paths (#26), not drive-relative ones', () => {
    for (const d of ['F:\\Github_Projects\\anyrpg', 'c:/code/r', '\\\\server\\share\\r']) expect(cloneDestProblem(d), d).toBeNull();
    for (const d of ['F:Github_Projects', 'repos\\r', '\\single']) expect(cloneDestProblem(d), d).not.toBeNull();
  });
});
