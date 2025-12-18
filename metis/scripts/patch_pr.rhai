//! Rough equivalent of templates/patch_pr.yaml.

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

echo "committing"
git commit -m 'task ${escape_single_quotes(metis_id)}'
echo "pushing"
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push origin "${branch_name}"

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
    let metis_id = env_or_default("METIS_ID", "unknown");
    let gh_token = env_or_default("GH_TOKEN", "");
    let prompt_input = env_or_default("PROMPT", "");
    let branch_name = `metis-${metis_id}`;
    let full_prompt = `${prompt_input}

Once you have finished with the prompt, please generate a pull request title and
description for this change. The title should go in .metis/output/pr_title.txt 
and the body should go in .metis/output/pr_body.txt .`;

    shell(`git checkout -b "${branch_name}"`, |setup_output| {
        codex(full_prompt, |codex_output| cleanup(metis_id, gh_token))
    })
}

run();
