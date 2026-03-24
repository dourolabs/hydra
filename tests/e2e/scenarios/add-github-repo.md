# Scenario: Verify Pre-Registered GitHub Repository

**ID:** add-github-repo
**Category:** core
**Priority:** P0
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 1 minute

## Description

Verify that the dourolabs/hydra-test-fixture repository was pre-registered during server bootstrap (by `run.sh`). This confirms the repository is available for all downstream test scenarios that depend on it.

## Steps (via dashboard)

1. Navigate to the repositories page at `http://localhost:8080`
2. Verify that `dourolabs/hydra-test-fixture` appears in the repository list
3. Confirm the repository entry shows the correct name and URL

## Expected Results

- The repositories page loads without errors
- `dourolabs/hydra-test-fixture` is listed as a registered repository
- No errors or broken UI elements are visible
