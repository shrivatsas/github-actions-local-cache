# Architecture intake

## Outcome

Provide reusable `restore` and `save` GitHub Actions for persistent, local filesystem caches on self-hosted runners.

## Constraints

- Multiple repositories/workflows; public action repository.
- Persistent storage is runner-provided; no cache service or runner patch.
- Cache content is an optimization, never an authority.
- Implementation waits for explicit approval after specification review.

## v1 boundary

Linux self-hosted runners only; repository-exclusive local POSIX mount; trusted code only; explicit restore/save lifecycle; no symlink preservation.

## Approval decisions outstanding

- Accept v1 resource limits and the no-symlink boundary.
- Confirm release-maintainer ownership and minimum supported Actions Runner version.
