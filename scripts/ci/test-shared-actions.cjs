const { test } = require('node:test');
const assert = require('node:assert/strict');
const { classifyTitle, applyMetadata } = require('../../.github/actions/pr-metadata/metadata.cjs');
const { reportBody, publishReport } = require('../../.github/actions/quality-report/report.cjs');
test('Conventional titles classify scopes and breaking changes', () => {
  for (const [title, label] of [
    ['fix(docs): improve diagrams', 'kind/documentation'],
    ['fix(security.auth)!: tighten access', 'kind/security'],
    ['feat: add a command', 'kind/feature'], ['perf(core): reduce allocations', 'kind/feature'],
    ['fix: handle missing files', 'kind/bug'], ['ci: check workflows', 'kind/chore'],
  ]) assert.deepEqual(classifyTitle(title), { valid: true, label });
  for (const title of ['Fix docs', 'fix:', 'feat(scope): ', 'fix(Scope): invalid']) assert.equal(classifyTitle(title).valid, false);
});
test('Validation without labels uses no write API', async () => {
  await applyMetadata({ github: null, context: { payload: { pull_request: { title: 'fix(docs): improve diagrams' } } }, core: { setFailed: assert.fail }, kindLabels: false });
});
test('Renaming a title replaces only managed labels', async () => {
  const calls = [];
  const issues = {
    listLabelsOnIssue: () => {}, removeLabel: async value => calls.push(['remove', value.name]),
    addLabels: async value => calls.push(['add', value.labels[0]]),
  };
  await applyMetadata({ github: { rest: { issues }, paginate: async () => [{ name: 'kind/bug' }, { name: 'priority/high' }] },
    context: { repo: { owner: 'org', repo: 'repo' }, issue: { number: 1 }, payload: { pull_request: { title: 'docs: clarify setup' } } },
    core: { setFailed: assert.fail }, kindLabels: true });
  assert.deepEqual(calls, [['remove', 'kind/bug'], ['add', 'kind/documentation']]);
});
test('Report reflects failure and stays below GitHub comment limit', () => {
  assert.match(reportBody('failure', 'partial evidence'), /## Ayni: failure/);
  assert.match(reportBody('success', 'evidence'), /## Ayni: passed/);
  assert.ok(reportBody('failure', 'x'.repeat(70000)).length < 65536);
});
test('Report updates only the established GitHub Actions comment', async () => {
  const calls = [];
  const issues = { listComments: () => {}, updateComment: async args => calls.push(args), createComment: assert.fail };
  await publishReport({ github: { rest: { issues }, paginate: async () => [
    { id: 1, user: { login: 'other[bot]' }, body: '<!-- ayni-signals -->' },
    { id: 2, user: { login: 'github-actions[bot]' }, body: '<!-- ayni-signals --> old' },
  ] }, context: { repo: { owner: 'org', repo: 'repo' }, issue: { number: 3 } }, reportPath: '/nonexistent/ayni.md', result: 'failure' });
  assert.equal(calls[0].comment_id, 2);
  assert.match(calls[0].body, /did not produce a report/);
});
