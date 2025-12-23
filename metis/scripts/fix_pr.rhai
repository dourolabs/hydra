//! Fetch PR feedback, apply fixes with Codex, and push updates to the branch.

fn fetch_feedback(next) {
    let body = `set -euo pipefail

mkdir -p .metis/output

if [ -z "${"${GH_TOKEN:-}"}" ]; then
  echo "GH_TOKEN not set; gh api may require authentication." >&2
fi

branch_name=$(git rev-parse --abbrev-ref HEAD)
pr_number=$(gh pr status --json number --jq '.currentBranch.number')

if [ -z "${"${pr_number}"}" ]; then
  echo "Unable to determine PR number for branch ${"${branch_name}"}." >&2
  exit 1
fi

headers=(-H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")

echo "Collecting review comments"
gh api "${"${headers[@]}"}" /repos/:owner/:repo/pulls/${"${pr_number}"}/comments --paginate \
  | jq -s 'flatten | map({id, pull_request_review_id, in_reply_to_id, body, commit_id, path, line, start_line, original_line, diff_hunk, created_at, updated_at, html_url, state, user: {login: .user.login}})' \
  > .metis/output/pr_feedback.json
  
gh api "${"${headers[@]}"}" /repos/:owner/:repo/pulls/${"${pr_number}"}/reviews --paginate \
  | jq -s 'flatten | map({id, body, commit_id, html_url, state, user: {login: .user.login}})' \
  > .metis/output/pr_reviews.json
`;

    shell(body, next)
}

fn cleanup(metis_id) {
    let cleanup_body = `set -euo pipefail

git add -A .

echo "checking diff"
if git diff --cached --quiet; then
  echo "No changes to commit; skipping push."
  exit 0
fi

echo "committing"
git commit -m 'task ${metis_id}'
echo "pushing"
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push --force-with-lease origin "${"${branch_name}"}"`;

    shell(cleanup_body, |output| output)
}

let prompt_input = env["PROMPT"] ?? "";
let metis_id = env["METIS_ID"] ?? "unknown";
if metis_id == "" {
    metis_id = "unknown";
}

let fix_prompt = prompt_input + `
You are responsible for addressing outstanding feedback on this pull request.
All PR line-by-line comments and overall reviews have been fetched with gh api and stored in
.metis/output/pr_feedback.json and .metis/output/pr_reviews.json. Read those files and make
the requested changes, adding or updating tests if needed. Work directly in this repository.

Once you have finished with the prompt, please generate a pull request title and
description for this change. The title should go in .metis/output/pr_title.txt 
and the body should go in .metis/output/pr_body.txt .`;

fetch_feedback(|feedback_output| {
    codex(fix_prompt, |codex_output| cleanup(metis_id))
});
