//! Rough equivalent of templates/fix_merge.yaml.

fn cleanup(metis_id) {
    let cleanup_body = `set -euo pipefail

if [ -z "${"${GH_TOKEN:-}"}" ]; then
  echo "GH_TOKEN not set; skipping push."
  exit 0
fi

git add -A 

git commit -m 'task ${metis_id}' || true
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push --force-with-lease origin "${"${branch_name}"}"`;
    shell(cleanup_body, |output| output)
}

fn run() {
    let onto = env["ONTO"] ?? "main";
    let metis_id = env["METIS_ID"] ?? "unknown";
    if onto == "" {
        onto = "main";
    }
    if metis_id == "" {
        metis_id = "unknown";
    }
    let prompt = "The current git repository is being rebased on to main and may contain conflicts.\nPlease fix the conflicts and complete the rebase. You may look at the git log to\nunderstand the history of changes and determine how to resolve the conflicts.";

    shell(`git merge ${onto} || true`, |merge_output| {
        codex(prompt, |codex_output| cleanup(metis_id))
    })
}

run();
