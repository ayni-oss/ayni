const fs = require('node:fs');
const marker = '<!-- ayni-signals -->';
function reportBody(result, report) {
  const status = result === 'success' ? 'passed' : result;
  if (report.length > 60000) report = `${report.slice(0, 60000)}\n\n_Report truncated; see the workflow log for the full result._\n`;
  return `${marker}\n## Ayni: ${status}\n\n${report}`;
}
async function publishReport({ github, context, reportPath, result }) {
  const report = fs.existsSync(reportPath) ? fs.readFileSync(reportPath, 'utf8')
    : 'Ayni did not produce a report. See the workflow log for details.\n';
  const body = reportBody(result, report);
  const issue = { ...context.repo, issue_number: context.issue.number };
  const comments = await github.paginate(github.rest.issues.listComments, { ...issue, per_page: 100 });
  const existing = comments.find(comment => comment.user?.login === 'github-actions[bot]' && comment.body?.includes(marker));
  if (existing) await github.rest.issues.updateComment({ ...context.repo, comment_id: existing.id, body });
  else await github.rest.issues.createComment({ ...issue, body });
}
module.exports = { reportBody, publishReport };
