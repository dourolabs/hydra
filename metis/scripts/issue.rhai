fn cleanup(metis_id) {
    let cleanup_body = `
set -euo pipefail

git add -A .

echo "checking diff"
if git diff --cached --quiet; then
  echo "No changes to commit; skipping PR creation."
  exit 0
fi

echo "committing"
git commit -m 'task $METIS_ID'
echo "pushing"
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push origin $branch_name

title=$(cat .metis/output/pr_title.txt)

echo "making PR"
gh pr create \
  --repo dourolabs/metis \
  --head $branch_name \
  --title "$title" \
  --body-file .metis/output/pr_body.txt`;

    shell(cleanup_body, |output| output)
}

let metis_id = env["METIS_ID"] ?? "unknown";
let issue_id = env["ISSUE_ID"] ?? "unknown";
let prompt_input = env["PROMPT"] ?? "";

let branch_name = `metis-${metis_id}`;
let full_prompt = `You are a software development agent working on an issue, with the goal of shipping a patch to resolve it.

You have access to several tools that enable you to do your job.

Please use the "metis issues" command to access the issue tracker -- either to get more information about your issue, or to create new follow-up issues that will be addressed later by yourself or other members of your team.

If you need to create a pull request, please use the "metis patches create --description 'description of change'" to create the PR from your current changes.
This command will return an id for the patch.
Then, please file a new issue of type merge-request where the text points to the id of the created patch.

While you are ultimately responsible for determining the best course of action,
here are some suggestions for the best way to resolve different types of issues:

**Feature**
Determine a plan of action for implementing the feature. 
If the plan seems small (single-PR sized), please implement the changes and submit a PR.
Otherwise, create new issues in the issue tracker (as children of the current issue) for the follow up items.
Please use the blocked-on dependency type to indicate any sequential dependencies between those tasks.

**Bug**
Debug the problem and identify the issue.
If the resolution to the issue seems single-PR sized, implement the changes and submit a PR.
Please try to include a test in this PR that verifies the solution to the bug.
Otherwise, create follow-up issues as children of this issue.
In this case, if feasible, please also create a PR with a test that isolates the bug and can be used to validate a fix.

**Task**
Follow the instructions in the task and submit a PR.

**Merge Request**



**Your issue id is:** ${issue_id}

Finally, generate a pull request title and description for this change. The title should go in .metis/output/pr_title.txt 
and the body should go in .metis/output/pr_body.txt .`;

shell(`git checkout -b "${branch_name}"`, |setup_output| {
    codex(full_prompt, |codex_output| cleanup(metis_id))
})
