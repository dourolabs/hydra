# Add a new repo

Requirements: the issue must specify the git remote to connect, eg. https://github.com/foo/bar.git

Required workflow:
0. Determine if this is the first time being run on this issue. Look at child issues to see if the steps in the "initial phase" have
   already been followed. If they have already been followed, skip to the iterative problem solving phase (step 8) below.

Initial Phase:   
1. Use "metis repos create" to add the repo to the system, eg "metis repos create foo/bar https://github.com/foo/bar.git" 
2. Clone the repository with "metis repos clone".
3. Create an issue to investigate the contents of the repository. The goal is to produce an index document for the repo in the document store
   under /repos/<repo-name>.md. The document must describe what is in the repository, its purpose, the names of major components, service names,
   module names, etc. It should include anything that would help a future agent determine when this repository needs to be used for a task.

   **Preferred method:** Write the index document directly to `$METIS_DOCUMENTS_DIR/repos/<repo-name>.md`. Changes are automatically
   pushed back to the document store when the job completes. Fall back to `metis documents put /repos/<repo-name>.md --file <file>`
   only if filesystem access is unavailable.
4. Create an issue to produce a docker image for the repository. The goal is to produce a PR adding "Dockerfile.metis" to the repository
   that contains all of the necessary dependencies for building / running / testing the code. The PR should also create a github action that
   automatically builds this image daily or on manual workflow trigger and pushes it to a container registry. Look at the repository to see
   how other docker images (if any) are built and pushed and follow the same pattern here.

   **Important: The Dockerfile must install the `metis` CLI binary so it is available on the agent's PATH.** Download the latest
   pre-built binary from the metis-releases repository. The binary URL uses a stable "latest" tag so the Dockerfile will always
   pull the most recent version. Add the following to the Dockerfile:

   ```dockerfile
   # Install the metis CLI binary
   RUN curl -fsSL https://github.com/dourolabs/metis-releases/releases/download/latest/metis-x86_64-unknown-linux-gnu \
       -o /usr/local/bin/metis \
       && chmod +x /usr/local/bin/metis
   ```

   This ensures the `metis` command is accessible from the CLI inside the container. The binary must be placed in a directory
   that is on the default PATH (e.g. `/usr/local/bin/`). Verify by adding a `RUN metis --version` step in the Dockerfile
   or by documenting that the agent should test `metis --version` after container startup.

5. Create an issue as a follow up to (4) to update the metis repo with the new image name. "metis repos update <repo-name> --default-image <image-name>"
6. Create an issue as a follow up to (5) that runs build / test / lint (as applicable) in the new repo. Ask the agent to report back any
   problems it encounters in the issue itself for further analysis.
7. End the session -- another agent will pick up this issue once the child issues above have completed.   

Iterative problem solving phase:
8. Inspect the results from the build / test / lint issue. Look at the issue itself, and if needed, look at logs from the agent run using
   "metis jobs logs <issue-id>". 
9. If the agent failed to successfully build / test / lint in the repo, determine if the docker image was the problem. If so, return to
   step (4) above and include instructions in the issue to address the problems.
10. Otherwise, the issue is done.   