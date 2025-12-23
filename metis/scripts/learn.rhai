//! Use feedback from another branch to harden AGENTS.md and open a PR with the updates.

fn fetch_feedback(target_branch, next) {
    let body = `set -euo pipefail

mkdir -p .metis/output

target_branch="${target_branch}"
if [ -z "$target_branch" ]; then
  echo "TARGET_BRANCH or BRANCH must be set to the branch with feedback." >&2
  exit 1
fi

if [ -z "$GH_TOKEN" ]; then
  echo "GH_TOKEN not set; gh api may require authentication." >&2
fi

pr_number=$(gh pr list --head "$target_branch" --json number --jq '.[0].number')
if [ -z "$pr_number" ]; then
  echo "Unable to find PR for branch $target_branch." >&2
  exit 1
fi

echo "Collecting review comments for $target_branch (PR #$pr_number)"
gh api -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28" /repos/:owner/:repo/pulls/$pr_number/comments --paginate \
  | jq -s 'flatten | map({id, pull_request_review_id, in_reply_to_id, body, commit_id, path, line, start_line, original_line, diff_hunk, created_at, updated_at, html_url, state, user: {login: .user.login}})' \
  > .metis/output/pr_feedback.json
gh api -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28" /repos/:owner/:repo/issues/$pr_number/comments --paginate \
  | jq -s 'flatten' \
  > .metis/output/pr_general_comments.json
gh api -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28" /repos/:owner/:repo/pulls/$pr_number/reviews --paginate \
  | jq -s 'flatten' \
  > .metis/output/pr_reviews.json
`;

    shell(body, next);
}

fn prepare_branch(branch_name, next) {
    let body = `set -euo pipefail

git checkout main
git checkout -b "${branch_name}" main
`;

    shell(body, next);
}

fn cleanup(branch_name, metis_id) {
    let cleanup_body = `set -euo pipefail

git add -A .

echo "checking diff"
if git diff --cached --quiet; then
  echo "No changes to commit; skipping PR creation."
  exit 0
fi

echo "committing"
git commit -m 'task ${metis_id}'
echo "pushing"
git push origin "${branch_name}"

title=$(cat .metis/output/pr_title.txt)

echo "making PR"
gh pr create \
  --repo dourolabs/metis \
  --head "${branch_name}" \
  --title "$title" \
  --body-file .metis/output/pr_body.txt`;
    shell(cleanup_body, |output| output);
}

fn run() {
    let prompt_input = env["PROMPT"] ?? "";
    let metis_id = env["METIS_ID"] ?? "unknown";
    if metis_id == "" {
        metis_id = "unknown";
    }

    let target_branch = env["TARGET_BRANCH"] ?? env["BRANCH"] ?? "";
    if target_branch == "" {
        target_branch = env["branch"] ?? "";
    }

    let work_branch = if metis_id == "unknown" {
        "learn-agents".to_string()
    } else {
        "learn-".to_string() + metis_id
    };

    let learn_prompt = `
You are improving AGENTS.md to prevent recurring issues flagged in PR feedback.
Context:
- Feedback source branch: ${target_branch}
- Review comments: .metis/output/pr_feedback.json
- PR reviews: .metis/output/pr_reviews.json
- General PR comments: .metis/output/pr_general_comments.json
- The user has also provided some additional feedback below

Additional Feedback
${prompt_input}

Tasks:
1) Read the feedback files to understand the root causes and expectations reviewers expressed.
2) Update AGENTS.md with concise, prescriptive guidance that would have prevented the issues.
   Keep existing structure when possible; add short subsections only when necessary.
   You may choose not to update AGENTS.md if it seems like there is nothing actionable in the feedback.
3) Make the guidance actionable for future contributors working on any branch, not just the one under review.
4) Capture the outcome in PR collateral: write .metis/output/pr_title.txt and .metis/output/pr_body.txt
   describing the AGENTS.md updates and the feedback they address.
Do not modify files other than AGENTS.md and the required PR output files unless strictly necessary.
`;

    prepare_branch(work_branch.clone(), |setup_output| {
        fetch_feedback(target_branch, |feedback_output| {
            codex(learn_prompt, |codex_output| cleanup(work_branch, metis_id))
        })
    });
}

run();
