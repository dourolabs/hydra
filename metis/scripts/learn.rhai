//! Use feedback from another branch to harden AGENTS.md and open a PR with the updates.

fn fetch_feedback(target_branch, next) {
    let body = "set -euo pipefail\n\n".to_string()
        + "mkdir -p .metis/output\n\n"
        + "target_branch=\"" + target_branch + "\"\n"
        + "if [ -z \"$target_branch\" ]; then\n"
        + "  echo \"TARGET_BRANCH or BRANCH must be set to the branch with feedback.\" >&2\n"
        + "  exit 1\n"
        + "fi\n\n"
        + "if [ -z \"${GH_TOKEN:-}\" ]; then\n"
        + "  echo \"GH_TOKEN not set; gh api may require authentication.\" >&2\n"
        + "fi\n\n"
        + "pr_number=$(gh pr list --head \"$target_branch\" --json number --jq '.[0].number')\n"
        + "if [ -z \"$pr_number\" ]; then\n"
        + "  echo \"Unable to find PR for branch $target_branch.\" >&2\n"
        + "  exit 1\n"
        + "fi\n\n"
        + "headers=(-H \"Accept: application/vnd.github+json\" -H \"X-GitHub-Api-Version: 2022-11-28\")\n"
        + "echo \"Collecting review comments for $target_branch (PR #$pr_number)\"\n"
        + "gh api \"${headers[@]}\" /repos/:owner/:repo/pulls/$pr_number/comments --paginate \\\n"
        + "  | jq -s 'flatten | map({id, pull_request_review_id, in_reply_to_id, body, commit_id, path, line, start_line, original_line, diff_hunk, created_at, updated_at, html_url, state, user: {login: .user.login}})' \\\n"
        + "  > .metis/output/pr_feedback.json\n"
        + "gh api \"${headers[@]}\" /repos/:owner/:repo/issues/$pr_number/comments --paginate \\\n"
        + "  | jq -s 'flatten' \\\n"
        + "  > .metis/output/pr_general_comments.json\n"
        + "gh api \"${headers[@]}\" /repos/:owner/:repo/pulls/$pr_number/reviews --paginate \\\n"
        + "  | jq -s 'flatten' \\\n"
        + "  > .metis/output/pr_reviews.json\n";

    shell(body, next)
}

fn prepare_branch(branch_name, next) {
    let body = "set -euo pipefail\n\n".to_string()
        + "git checkout main\n"
        + "git checkout -b \"" + branch_name + "\" main\n";

    shell(body, next)
}

fn cleanup(branch_name, metis_id) {
    let cleanup_body = "set -euo pipefail\n\n".to_string()
        + "git add -A .\n\n"
        + "echo \"checking diff\"\n"
        + "if git diff --cached --quiet; then\n"
        + "  echo \"No changes to commit; skipping PR creation.\"\n"
        + "  exit 0\n"
        + "fi\n\n"
        + "echo \"committing\"\n"
        + "git commit -m 'task " + metis_id + "'\n"
        + "echo \"pushing\"\n"
        + "git push origin \"" + branch_name + "\"\n\n"
        + "title=$(cat .metis/output/pr_title.txt)\n\n"
        + "echo \"making PR\"\n"
        + "gh pr create \\\n"
        + "  --repo dourolabs/metis \\\n"
        + "  --head \"" + branch_name + "\" \\\n"
        + "  --title \"$title\" \\\n"
        + "  --body-file .metis/output/pr_body.txt";

    shell(cleanup_body, |output| output)
}

let prompt_input = env["PROMPT"] ?? "";
let mut metis_id = env["METIS_ID"] ?? "unknown";
if metis_id == "" {
    metis_id = "unknown";
}

let mut target_branch = env["TARGET_BRANCH"] ?? env["BRANCH"] ?? "";
if target_branch == "" {
    target_branch = env["branch"] ?? "";
}

let work_branch = if metis_id == "unknown" {
    "learn-agents".to_string()
} else {
    "learn-".to_string() + metis_id
};

let learn_prompt = prompt_input
    + "\n\nYou are improving AGENTS.md to prevent recurring issues flagged in PR feedback.\n"
    + "Context:\n"
    + "- Feedback source branch: "
    + target_branch
    + "\n"
    + "- Review comments: .metis/output/pr_feedback.json\n"
    + "- PR reviews: .metis/output/pr_reviews.json\n"
    + "- General PR comments: .metis/output/pr_general_comments.json\n"
    + "\n"
    + "Tasks:\n"
    + "1) Read the feedback files to understand the root causes and expectations reviewers expressed.\n"
    + "2) Update AGENTS.md with concise, prescriptive guidance that would have prevented the issues.\n"
    + "   Keep existing structure when possible; add short subsections only when necessary.\n"
    + "3) Make the guidance actionable for future contributors working on any branch, not just the one under review.\n"
    + "4) Capture the outcome in PR collateral: write .metis/output/pr_title.txt and .metis/output/pr_body.txt\n"
    + "   describing the AGENTS.md updates and the feedback they address.\n"
    + "Do not modify files other than AGENTS.md and the required PR output files unless strictly necessary.\n";

prepare_branch(work_branch.clone(), |setup_output| {
    fetch_feedback(target_branch, |feedback_output| {
        codex(learn_prompt, |codex_output| cleanup(work_branch, metis_id))
    })
})
