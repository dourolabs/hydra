# Design document review

A playbook for agents that need to produce and review design documents before implementing a feature or significant piece of work.

## When to use this playbook

Use this playbook whenever an issue requires a design document before implementation. This includes:

- New features with non-trivial scope
- Architectural changes that affect multiple components
- Work where the approach needs stakeholder sign-off before coding begins

## Prerequisites

- The agent has an assigned issue (available as `$METIS_ISSUE_ID`).
- The issue description specifies the work that needs a design document.

## Workflow

### Phase 1: Write the design document

1. Research the problem space. Clone relevant repositories, read existing code, and gather context needed to write the design.
2. Write a design document in markdown covering:
   - **Problem statement:** What problem does this solve?
   - **Goals and non-goals:** What is in and out of scope?
   - **Proposed approach:** How will the work be done? Include key design decisions, trade-offs, and alternatives considered.
   - **Key changes:** Which files, modules, or services are affected?
   - **Risks and open questions:** What might go wrong or still needs clarification?
3. Publish the design document to the document store under `/designs/`:

   **Preferred method:** Write the design document directly to `$METIS_DOCUMENTS_DIR/designs/<feature-slug>.md`. Changes are
   automatically pushed back to the document store when the job completes. Use a descriptive slug for the filename
   (e.g., `user-auth-redesign.md`).

   **Alternative (CLI):** If filesystem access is unavailable, use the CLI:

   ```bash
   metis documents create \
     --title "<Feature name> design" \
     --path "/designs/<feature-slug>.md" \
     --body-file design.md
   ```

### Phase 2: Request review

4. Create a review issue assigned to the design's creator (the person who filed the original issue). The review issue should be a child of the current issue:

   ```bash
   metis issues create \
     "Review design document: <Feature name>

   Please review the design document at /designs/<feature-slug>.md

   ## How to respond
   - If the design is **approved**, close this issue:
     metis issues update <this-review-issue-id> --status closed
   - If the design is **rejected**, mark this issue as failed and record your feedback in progress notes:
     metis issues update <this-review-issue-id> --status failed --progress 'Feedback: ...'

   ## Design document link
   Path: /designs/<feature-slug>.md
   Read it from the filesystem: cat \$METIS_DOCUMENTS_DIR/designs/<feature-slug>.md
   Or via CLI: metis documents get --path /designs/<feature-slug>.md" \
     --assignee <creator-username> \
     --deps child-of:$METIS_ISSUE_ID
   ```

5. End the session. The agent must wait for the reviewer to act on the review issue before proceeding.

### Phase 3: Handle the review outcome

6. When the agent is re-invoked on the parent issue, inspect the review issue's status:
   - Check child issues: `metis issues describe $METIS_ISSUE_ID` and look at children.
   - Read the review issue: `metis issues describe <review-issue-id>`.

7. **If the review issue status is `closed` (approved):**
   - The design is approved. Proceed to Phase 4 (implementation planning).

8. **If the review issue status is `failed` (rejected):**
   - Read the reviewer's feedback from the review issue's progress field.
   - Revise the design document to address the feedback.
   - Publish the updated document under `/designs/`:

     **Preferred method:** Edit the file directly at `$METIS_DOCUMENTS_DIR/designs/<feature-slug>.md`. Changes are
     automatically pushed back to the document store when the job completes.

     **Alternative (CLI):** If filesystem access is unavailable:

     ```bash
     metis documents update <document-id> --body-file revised-design.md
     ```

   - Create a new review issue following step 4 above, assigned to the same reviewer.
   - End the session and wait for the new review.

9. **If the review issue is still open or in-progress:**
   - The reviewer has not yet responded. End the session and wait.

### Phase 4: Proceed with implementation

10. Once the design is approved, continue with the normal planning and issue-creation flow:
    - Break the approved design into PR-sized implementation tasks.
    - Create child issues for each task with appropriate dependencies.
    - Update the parent issue's progress field with the implementation plan and task list.

## Summary of issue statuses used in review

| Status | Meaning |
|--------|---------|
| `open` | Review issue created, awaiting reviewer action |
| `in-progress` | Reviewer is actively reviewing |
| `closed` | Design **approved** — proceed with implementation |
| `failed` | Design **rejected** — revise and re-submit |

## Notes

- The reviewer records feedback in the review issue's progress field regardless of outcome (approval or rejection).
- Each revision cycle creates a new review issue so there is a clear audit trail of review rounds.
- The agent must never proceed to implementation without an approved (closed) review issue.