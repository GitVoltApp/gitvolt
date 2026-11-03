import { describe, expect, it } from 'vitest';
import { finishedText } from './actions';

describe('git flow finish', () => {
  it('says what it did, in one sentence', () => {
    expect(finishedText('feature/login', { status: 'done', merged: ['develop'], tag: null, deleted: true })).toBe('Finished feature/login: merged into develop and deleted');
    expect(finishedText('release/1.2.0', { status: 'done', merged: ['main', 'develop'], tag: 'v1.2.0', deleted: false })).toBe('Finished release/1.2.0: merged into main and develop, and tagged v1.2.0');
    expect(finishedText('hotfix/1.0.1', { status: 'done', merged: [], tag: null, deleted: false })).toBe('hotfix/1.0.1 is already finished');
  });
});
