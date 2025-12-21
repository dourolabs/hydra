//! Fetch PR feedback, apply fixes with Codex, and push updates to the branch.

fn fetch_feedback(next) {
    let body = `set -euo pipefail

mkdir -p .metis/output

if [ -z "${"${GH_TOKEN:-}"}" ]; then
  echo "GH_TOKEN not set; gh pr view may require authentication." >&2
fi

echo "Collecting PR comments and reviews"
gh pr view --json comments,reviews,author,number,title --jq '.' > .metis/output/pr_feedback.json`;

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
All PR comments and reviews have been fetched with gh pr view and stored in
.metis/output/pr_feedback.json. Read that file and make the requested changes,
adding or updating tests if needed. Work directly in this repository.

Once you have finished with the prompt, please generate a pull request title and
description for this change. The title should go in .metis/output/pr_title.txt 
and the body should go in .metis/output/pr_body.txt .`;

fetch_feedback(|feedback_output| {
    codex(fix_prompt, |codex_output| cleanup(metis_id))
});
