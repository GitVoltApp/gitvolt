import { describe, expect, it } from 'vitest';
import { bareHost, detectHostKind, HOST_KIND_NAMES } from './hostKind';

describe('detectHostKind (the Rust host_kind)', () => {
  it('knows GitHub, GitLab and any host with gitlab in a label', () => {
    expect(detectHostKind('github.com')).toBe('github');
    expect(detectHostKind('GitLab.com')).toBe('gitlab');
    expect(detectHostKind('gitlab.example.org')).toBe('gitlab');
    expect(detectHostKind('code.example.com')).toBe('generic');
    expect(HOST_KIND_NAMES.generic).toBe('Generic');
  });

  it('knows Forgejo by Codeberg or a forgejo/gitea label, after GitLab', () => {
    expect(detectHostKind('codeberg.org')).toBe('forgejo');
    expect(detectHostKind('Codeberg.org:2222')).toBe('forgejo');
    expect(detectHostKind('forgejo.example.org')).toBe('forgejo');
    expect(detectHostKind('code.mygitea.example.com:3000')).toBe('forgejo');
    expect(detectHostKind('gitlab.forgejo.example.com')).toBe('gitlab');
    expect(detectHostKind('codeberg.org.example.com')).toBe('generic');
    expect(HOST_KIND_NAMES.forgejo).toBe('Forgejo');
  });
});

describe('bareHost (the core normalize_host)', () => {
  it('reads a pasted address as its host: no scheme, path or query, lowercase', () => {
    expect(bareHost('https://Git.Example.dev/')).toBe('git.example.dev');
    expect(bareHost(' git.example.dev:3000/user/settings ')).toBe('git.example.dev:3000');
    expect(bareHost('git.example.dev?x=1')).toBe('git.example.dev');
    expect(bareHost('')).toBe('');
  });
});
