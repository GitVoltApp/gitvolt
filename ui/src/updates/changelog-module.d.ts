// CHANGELOG.md's released sections, made at build time by build/changelog.ts.
declare module 'virtual:gitvolt-changelog' {
  const entries: import('./changelog').ChangelogEntry[];
  export default entries;
}
