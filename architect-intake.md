# Architecture intake

## Outcome

Provide reusable `restore` and `save` GitHub Actions for persistent, local filesystem caches on self-hosted runners.

## Constraints

- Multiple repositories/workflows; public action repository.
- Persistent storage is runner-provided; no cache service or runner patch.
- Cache content is an optimization, never an authority.
- Implementation was approved on 2026-08-28 and uses a Rust cache engine behind dependency-free Node 24 launchers.

## v1 boundary

Linux self-hosted runners only; repository-exclusive local POSIX mount; trusted code only; explicit restore/save lifecycle; no symlink preservation.

## Release decisions outstanding

- Confirm release-maintainer ownership before publishing v1.
- The minimum supported Actions Runner version is v2.328.0.
