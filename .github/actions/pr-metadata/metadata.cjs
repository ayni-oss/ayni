const pattern = /^(build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(?:\(([a-z0-9][a-z0-9._/-]*)\))?(!)?: \S(?:.*\S)?$/;
const managedLabels = ['kind/bug', 'kind/chore', 'kind/documentation', 'kind/feature', 'kind/security'];

function classifyTitle(title) {
  const match = title.trim().match(pattern);
  if (!match) return { valid: false, label: null };
  const [, type, scope = ''] = match;
  const scopeRoot = scope.split(/[./]/, 1)[0];
  const label = scopeRoot === 'security' ? 'kind/security'
    : type === 'docs' || scopeRoot === 'docs' ? 'kind/documentation'
    : type === 'feat' || type === 'perf' ? 'kind/feature'
    : type === 'fix' ? 'kind/bug' : 'kind/chore';
  return { valid: true, label };
}

async function applyMetadata({ github, context, core, kindLabels }) {
  const title = context.payload.pull_request.title.trim();
  const { valid, label } = classifyTitle(title);
  if (kindLabels) {
    const issue = { ...context.repo, issue_number: context.issue.number };
    const current = await github.paginate(github.rest.issues.listLabelsOnIssue, { ...issue, per_page: 100 });
    const names = new Set(current.map(({ name }) => name));
    for (const name of managedLabels) {
      if (name !== label && names.has(name)) await github.rest.issues.removeLabel({ ...issue, name });
    }
    if (label && !names.has(label)) await github.rest.issues.addLabels({ ...issue, labels: [label] });
  }
  if (!valid) core.setFailed(`PR title must match Conventional Commits; received: ${title}`);
}
module.exports = { classifyTitle, applyMetadata };
