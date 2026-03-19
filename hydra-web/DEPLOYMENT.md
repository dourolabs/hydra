# Deploying hydra-web

## Overview

The hydra-web frontend is a React SPA that is served in production by the Rust `hydra-bff` crate. The BFF handles authentication, API proxying, and static asset serving.

The `hydra-web.Dockerfile` and its CI workflow have been removed. The frontend SPA assets are now embedded into the `hydra-bff` binary at build time.

For Kubernetes deployment of the BFF, see the `hydra-bff` crate documentation and the `dourolabs/hydra-cluster` repository.
