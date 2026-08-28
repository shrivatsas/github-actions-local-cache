# v1 specification review record

Status: **changes incorporated; implementation approved on 2026-08-28; release ownership still to be assigned.**

| Dimension | First-pass result | Required changes incorporated |
| --- | --- | --- |
| Security | Blocked | Same-UID trust limit, repository-exclusive root, untrusted-branch write exclusion, root validation, no links, strict metadata, resource limits, staging, and quarantine. |
| Durability | Blocked | Explicit O_EXCL lock protocol, fsync/publish order, immutable entries, winner validation, and runner-drained retention. |
| Performance/operations | Conditional | 256-entry fallback bound, fixed limits, quota/inode requirements, stable diagnostics, fault tests, and benchmarks. |
| Ease of use | Conditional | Explicit restore/save usage, deterministic glob grammar, conflict behavior, compact diagnostics, and host preflight. |
| Extensibility/upgrades | Conditional | Versioned schema, compatibility fields, SHA pinning, protected releases, provenance, and coexistence/retention policy. |

## Decisions made

1. No fork of the obsolete local-cache action: v1 is a clean Rust design with dependency-free Node 24 launchers and no host shell or compression-tool dependency.
2. Two explicit actions, not a post-action wrapper: `save` runs after all gates under `if: success()`.
3. No shared cache root or symlink preservation in v1. This fits immutable artifact and package-download-cache workloads, not `node_modules`.
4. Host retention drains its runner rather than introducing a cache daemon or fragile lease protocol.

## Approval record

- The Linux/self-hosted/repository-exclusive scope, explicit lifecycle, fixed limits, and no-symlink rule were approved for implementation on 2026-08-28.
- Actions Runner v2.328.0 is the minimum supported release.
- Release-maintainer ownership must be confirmed before publishing v1.
