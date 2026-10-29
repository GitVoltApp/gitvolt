import { describe, expect, it } from 'vitest';
import { forgeName, mrLongNoun, mrName, mrNoun, mrRef, mrSectionLabel, pipelineNoun } from './labels';

describe('forge labels: the only place the UI branches on the forge', () => {
  it('says MR and !n for GitLab, PR and #n for GitHub', () => {
    expect([forgeName('gitlab'), mrNoun('gitlab'), mrRef('gitlab', 12), mrSectionLabel('gitlab')]).toEqual(['GitLab', 'MR', '!12', 'Merge requests']);
    expect([forgeName('github'), mrNoun('github'), mrRef('github', 12), mrSectionLabel('github')]).toEqual(['GitHub', 'PR', '#12', 'Pull requests']);
  });

  it("says GitHub's words for Forgejo, under its own name", () => {
    expect([forgeName('forgejo'), mrNoun('forgejo'), mrRef('forgejo', 12), mrSectionLabel('forgejo'), mrName('forgejo'), pipelineNoun('forgejo')]).toEqual(['Forgejo', 'PR', '#12', 'Pull requests', 'Pull request', 'Checks']);
  });
});

// --- 4C T6 ---
it('names the whole noun for menus and buttons', () => {
  expect([mrLongNoun('gitlab'), mrLongNoun('github')]).toEqual(['merge request', 'pull request']);
});
// --- end 4C T6 ---
