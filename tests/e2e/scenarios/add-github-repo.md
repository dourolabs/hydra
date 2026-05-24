# Scenario: Verify Pre-Registered GitHub Repository

**ID:** add-github-repo
**Category:** core
**Priority:** P0
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 1 minute

## Description

Verify that the `dourolabs/hydra-test-fixture` repository was pre-registered during server bootstrap (by `run.sh`) **and** that the merge policy applied by `run.sh` (required `reviewer` approval, anyone may merge) was persisted. This confirms the repository is available for all downstream test scenarios and that the merge-time-constraints workflow has teeth in e2e — without the policy, `hydra patches merge` would succeed against an empty merge policy and the reviewer-agent / agent-handoff scenarios would not exercise the dry-run -> review-request -> retry ladder.

## Steps (via dashboard)

1. Navigate to the repositories page at `http://localhost:8080`
2. Verify that `dourolabs/hydra-test-fixture` appears in the repository list
3. Confirm the repository entry shows the correct name and URL
4. Validate the merge policy was applied by `run.sh`. The dashboard does not currently surface `merge_policy` in the repositories UI (separate finding — flag in the test report, do not fix as part of this scenario). As a fallback, query the API directly via bash:
   ```bash
   env -u HYDRA_TOKEN HYDRA_SERVER_URL=http://127.0.0.1:8080 \
     ./target/release/hydra-sp repos list --output json \
     | jq '.[] | select(.name == "dourolabs/hydra-test-fixture") | .repository.merge_policy'
   ```
   And verify the returned JSON contains:
   - `reviewers[0].any_of` includes `"reviewer"` — the required reviewer is the `reviewer` agent.
   - `mergers` is absent / null — no merger restriction, anyone may merge.

## Expected Results

- The repositories page loads without errors
- `dourolabs/hydra-test-fixture` is listed as a registered repository
- No errors or broken UI elements are visible
- The API-level `merge_policy` JSON for the fixture repository reflects what `run.sh` set:
  - At least one reviewer group requires the `reviewer` agent.
  - `mergers` is unset (anyone may merge once the reviewer-approval condition is satisfied).
- The dashboard not surfacing `merge_policy` is flagged in the test report as a separate finding (UI gap), not as a failure of this scenario.
