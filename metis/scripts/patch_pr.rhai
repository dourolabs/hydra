fn cleanup(metis_id) {
    let cleanup_body = `
set -euo pipefail

git add -A .

echo "checking diff"
if git diff --cached --quiet; then
  echo "No changes to commit; skipping PR creation."
  exit 0
fi

title=$(cat .metis/output/pr_title.txt)

echo "committing"
git commit -m "$title"
echo "pushing"
branch_name=$(git rev-parse --abbrev-ref HEAD)
git push origin $branch_name

echo "making PR"
gh pr create \
  --repo dourolabs/metis \
  --head $branch_name \
  --title "$title" \
  --body-file .metis/output/pr_body.txt`;

    shell(cleanup_body, |output| output)
}

let metis_id = env["METIS_ID"] ?? "unknown";
if metis_id == "" {
    metis_id = "unknown";
}
let prompt_input = env["PROMPT"] ?? "";

let branch_name = `metis-${metis_id}`;
let full_prompt = `${prompt_input}

Once you have finished with the prompt, please generate a pull request title and
description for this change. The title should go in .metis/output/pr_title.txt 
and the body should go in .metis/output/pr_body.txt .`;

shell(`git checkout -b "${branch_name}"`, |setup_output| {
    codex(full_prompt, |codex_output| cleanup(metis_id))
})
