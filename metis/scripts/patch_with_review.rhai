//! Rough equivalent of templates/patch_with_review.yaml.

fn env_or_default(key, default_value) {
    if env.contains(key) {
        let value = env[key];
        if value == "" {
            return default_value;
        }
        return value;
    }

    default_value
}

fn escape_single_quotes(input) {
    input.replace("'", "'\"'\"'")
}

fn snapshot_patch(next) {
    // We approximate the parent patch by capturing the current git diff because a single
    // script does not have task-level context like the workflow runner.
    let body = "mkdir -p .metis/generate-patch .metis/parents/generate-patch\n\
git diff > .metis/generate-patch/changes.patch\n\
cp .metis/generate-patch/changes.patch .metis/parents/generate-patch/changes.patch";
    let command = `bash -lc '${escape_single_quotes(body)}'`;
    shell(command, next)
}

fn copy_review_outputs(next) {
    let body = "mkdir -p .metis/parents/code-review\n\
if [ -d .metis/output ]; then\n\
  cp -R .metis/output/. .metis/parents/code-review/\n\
fi";
    let command = `bash -lc '${escape_single_quotes(body)}'`;
    shell(command, next)
}

fn cleanup(metis_id, gh_token) {
    if gh_token == "" {
        print("GH_TOKEN not set; skipping PR creation.");
        return "Skipped PR creation";
    }

    let cleanup_body = `set -euo pipefail

export GH_TOKEN='${escape_single_quotes(gh_token)}'

git add -A 

echo "checking diff"
if git diff --cached --quiet; then
  echo "No changes to commit; skipping PR creation."
  exit 0
fi

echo "remotes"
git remote -v

echo "committing"
git commit -m 'task ${escape_single_quotes(metis_id)}'
echo "pushing"
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push --force-with-lease origin "${branch_name}"

title=$(cat .metis/output/pr_title.txt)

echo "making PR"
gh pr create \
  --repo dourolabs/metis \
  --head "${branch_name}" \
  --title "${title}" \
  --body-file .metis/output/pr_body.txt`;

    let command = `bash -lc '${escape_single_quotes(cleanup_body)}'`;
    shell(command, |output| output)
}

fn run() {
    let prompt_input = env_or_default("PROMPT", "");
    let metis_id = env_or_default("METIS_ID", "unknown");
    let gh_token = env_or_default("GH_TOKEN", "");

    let review_prompt = "You are a code review agent who is responsible for approving or \n\
rejecting the changes from a contributor. Please look at the changes in .metis/generate-patch/changes.patch \n\
as being applied to the current repository. \n\
\n\
If the code *does not* meet your standards, please write the reasons it does not \n\
into the .metis/output/error.txt file. If the code looks good, do not generate this file.\n\
\n\
The task provided to the prior contributor was as follows:\n\
\n\
" + prompt_input;

    let fix_prompt = "You are assigned to fix problems with a given patch to the provided repository. \n\
A code reviewer has looked at the patch and provided a review.\n\
\n\
The patch is available at .metis/parents/generate-patch/changes.patch and the \n\
code review is in the text files under .metis/parents/code-review/\n\
\n\
Once you've finished fixing problems, please generate a pull request title and\n\
description for this change. The title should go in .metis/output/pr_title.txt \n\
and the body should go in .metis/output/pr_body.txt .";

    shell(`echo "this is the setup"`, |setup_output| {
        codex(prompt_input, |generate_output| {
            snapshot_patch(|_| {
                codex(review_prompt, |review_output| {
                    copy_review_outputs(|_| {
                        codex(fix_prompt, |fix_output| cleanup(metis_id, gh_token))
                    })
                })
            })
        })
    })
}

run();
