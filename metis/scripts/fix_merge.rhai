//! Rough equivalent of templates/fix_merge.yaml.

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

git commit -m 'task ${escape_single_quotes(metis_id)}' || true
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push --force-with-lease origin "${branch_name}"`;
    let command = `bash -lc '${escape_single_quotes(cleanup_body)}'`;

    shell(command, |output| output)
}

fn run() {
    let onto = env_or_default("ONTO", "main");
    let metis_id = env_or_default("METIS_ID", "unknown");
    let gh_token = env_or_default("GH_TOKEN", "");
    let prompt = "The current git repository is being rebased on to main and may contain conflicts.\nPlease fix the conflicts and complete the rebase. You may look at the git log to\nunderstand the history of changes and determine how to resolve the conflicts.";

    shell(`git merge ${onto} || true`, |merge_output| {
        codex(prompt, |codex_output| cleanup(metis_id, gh_token))
    })
}

run();
