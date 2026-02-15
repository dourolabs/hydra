# dourolabs/metis-releases - Repository Index

## Overview

**dourolabs/metis-releases** is a repository dedicated to hosting published binary release artifacts for the Metis platform. It serves as a distribution point for pre-built binaries and does not contain any source code, buildable components, or application logic.

## Repository Purpose

- **Binary artifact hosting:** Stores published release binaries (e.g., CLI tools, server binaries) for distribution.
- **No source code:** This repository contains no source code, libraries, or modules. All source code lives in other repositories (primarily `dourolabs/metis`).
- **No build system:** There are no build scripts, Makefiles, Cargo.toml, package.json, or any other build configuration. Artifacts are built elsewhere and published here.
- **No Docker image needed:** Since there is nothing to build or run from this repository, no Docker image or CI build pipeline is required.

## Current State

The repository is essentially empty, containing only a placeholder `README.md`. It is intended to be populated with binary release artifacts as the Metis project publishes new versions.

## Repository Structure

```
metis-releases/
└── README.md    # Placeholder README
```

## Relevance for Agents

- **Do not attempt to build, test, or lint this repository.** There is no code to compile or validate.
- **Do not create a Docker image** for this repository. It holds static binary artifacts only.
- **Release publishing:** When the Metis project produces new release binaries, they should be published to this repository (or as GitHub Releases attached to this repository).
- **Downstream consumption:** Other repositories or deployment pipelines may pull binaries from this repository's releases.

## Related Repositories

| Repository | Relationship |
|-----------|-------------|
| `dourolabs/metis` | Source code repository; builds are produced here and published to metis-releases |
| `dourolabs/metis-cluster` | Kubernetes cluster infrastructure that may consume released binaries |