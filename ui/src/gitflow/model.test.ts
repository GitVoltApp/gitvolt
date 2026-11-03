import { describe, expect, it } from 'vitest';
import type { GitflowConfig } from '../api/gen/GitflowConfig';
import { baseOf, classify, configError, defaultConfig, startNameError, suggestedTag, tagOf, targetsOf } from './model';

const config: GitflowConfig = { master: 'main', develop: 'develop', feature: 'feature/', release: 'release/', hotfix: 'hotfix/', versionTag: 'v' };

describe('git flow model', () => {
  it('classifies a branch by its prefix, as the core does', () => {
    expect(classify(config, 'feature/login')).toEqual({ kind: 'feature', rest: 'login' });
    expect(classify(config, 'release/1.2.0')).toEqual({ kind: 'release', rest: '1.2.0' });
    expect(classify(config, 'hotfix/1.2.1')).toEqual({ kind: 'hotfix', rest: '1.2.1' });
    expect(classify(config, 'feature/')).toBeNull();
    expect(classify(config, 'develop')).toBeNull();
  });

  it('a hotfix starts from production and goes back to both; a feature lives on development', () => {
    expect([baseOf(config, 'feature'), baseOf(config, 'release'), baseOf(config, 'hotfix')]).toEqual(['develop', 'develop', 'main']);
    expect(targetsOf(config, 'feature')).toEqual(['develop']);
    expect(targetsOf(config, 'hotfix')).toEqual(['main', 'develop']);
    expect([tagOf(config, 'feature', 'x'), tagOf(config, 'release', '1.2.0')]).toEqual([null, 'v1.2.0']);
  });

  it('Finish proposes the version a release or hotfix is named after as its tag', () => {
    const plain = { ...config, versionTag: '' };
    expect(suggestedTag(plain, 'release', '3.0.0')).toBe('3.0.0');
    expect(suggestedTag(plain, 'release', 'v3.0.0')).toBe('3.0.0');
    expect(suggestedTag(plain, 'hotfix', '3.0.1-rc.1')).toBe('3.0.1-rc.1');
    expect(suggestedTag(plain, 'release', '3.0')).toBe('3.0');
    expect(suggestedTag(config, 'release', '3.0.0')).toBe('v3.0.0');
    expect(suggestedTag(plain, 'release', 'spring-launch')).toBe('');
    expect(suggestedTag(plain, 'feature', '3.0.0')).toBeNull();
  });

  it("Init suggests the production branch the repository has", () => {
    expect(defaultConfig(['master', 'dev']).master).toBe('master');
    expect(defaultConfig(['main', 'master']).master).toBe('main');
    expect(defaultConfig([]).master).toBe('main');
  });

  it("says why settings can't be used, in the core's words", () => {
    expect(configError(config)).toBeNull();
    expect(configError({ ...config, develop: 'main' })).toBe('The production and development branches must differ');
    expect(configError({ ...config, release: '' })).toBe('Enter a release prefix');
    expect(configError({ ...config, hotfix: 'feature/' })).toBe('The hotfix and feature prefixes overlap');
    expect(configError({ ...config, feature: 'dev' })).toBe('The feature prefix would make develop a feature branch');
    expect(configError({ ...config, versionTag: 'v..' })).toMatch(/^The version tag prefix: /);
  });

  it("a new branch's name is checked with its prefix on", () => {
    const taken = (b: string) => b === 'feature/login';
    expect(startNameError(config, 'feature', '', taken)).toBe('Enter a feature name');
    expect(startNameError(config, 'release', '', taken)).toBe('Enter a version');
    expect(startNameError(config, 'feature', 'login', taken)).toBe('A branch named feature/login already exists');
    expect(startNameError(config, 'feature', 'a b', taken)).toMatch(/spaces/);
    expect(startNameError(config, 'release', '1.3.0', taken)).toBeNull();
  });
});
