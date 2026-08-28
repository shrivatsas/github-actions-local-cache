# Local Cache Action — v1 specification

## Purpose and boundary

`github-actions-local-cache` provides two JavaScript GitHub Actions—`restore` and `save`—for persistent local filesystem caches on Linux self-hosted runners. Workflows restore before use and place save at the end of a successful gate; this avoids a post-action save that cannot prove later steps passed.

It is not a remote cache service, runner modification, package-manager installer, or retention daemon. The runner operator supplies the mounted cache root, access controls, quota, and retention. Cache contents are untrusted optimization data, never authority for source, release, or security decisions.

## Security model

- The action defends against malformed/torn archives and accidental corruption. It does not provide confidentiality, authenticity, or isolation from code running as the same OS UID with access to the cache root or workspace.
- Use an exclusive repository-scoped cache mount, owned by the runner user and inaccessible to other repositories/users. Untrusted and unreviewed PR branches must have no writable access.
- User keys never become filenames or shell fragments; entry addresses use SHA-256. The action never runs a shell.
- Workflow-level integrity checks remain mandatory where cached artifacts influence correctness or provenance.

## Supported envelope and setup

v1 supports Linux self-hosted `linux-x64` and `linux-arm64`, a local POSIX filesystem, and a documented minimum Actions Runner release that supports JavaScript `node24`. It has no network calls and needs no GitHub token.

The root is supplied by `cache-dir` or `CACHE_DIR`; absence is an error. It must be absolute, existing, non-symlinked, outside `GITHUB_WORKSPACE`, owned by the runner user with mode `0700`. Root components are verified without following symlinks. Entries are also namespaced by immutable repository numeric ID for organization and collision avoidance; this is defense in depth, not a security boundary, and never makes a shared root acceptable. Windows, macOS, NFS/SMB, containers without the mount, shared roots, and cross-OS caching are excluded.

Operators provision persistent storage plus byte/inode quota and drain the runner before retention. v1 does not introduce a cache daemon or coordinate cleanup with active jobs.

## Interface

```yaml
- id: local-cache
  uses: shrivatsas/github-actions-local-cache/restore@<full-commit-sha>
  with:
    path: fixtures/extraction/demo-seeds/*.json.gz
    key: demo-bootstrap-extraction-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles(...) }}
    cache-dir: /media/cache/actions

# Run guards and consumer gates here.

- if: ${{ success() && steps.local-cache.outputs.cache-match != 'exact' }}
  uses: shrivatsas/github-actions-local-cache/save@<full-commit-sha>
  with:
    path: fixtures/extraction/demo-seeds/*.json.gz
    key: demo-bootstrap-extraction-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles(...) }}
    cache-dir: /media/cache/actions
```

Shared inputs: `path` (required workspace-relative newline-delimited literal/glob list), `key` (required, 1–512 UTF-8 bytes), `cache-dir` (optional, overrides `CACHE_DIR`), and `fail-on-cache-error` (default `true`). Restore additionally accepts opt-in ordered `restore-keys` prefixes.

Restore outputs: `cache-hit` is true only for exact hit; `cache-match` is `exact`, `fallback`, `miss`, or `error`. Save output: `cache-save` is `saved`, `raced`, `skipped-no-paths`, or `error`. Outputs are diagnostic only and never contain raw keys or paths.

Lists accept LF/CRLF and ignore blank lines. Boolean values are exactly `true` or `false`; empty `cache-dir` is unset; globs cannot negate.

At save, matches are workspace-relative POSIX paths, lexically sorted, de-duplicated, and recorded as expanded paths. Directories recurse and empty directories remain. No matches is `skipped-no-paths`.

At restore, an existing destination causes a fail-closed error; the action never overwrites workspace content.

## Entry format and lifecycle

```text
<cache-root>/v1/<repository-id>/<sha256(key)>/
  metadata.json
  payload.tar.zst
  complete
```

Strict, ≤1 MiB metadata includes schema, original key, creation timestamp, payload byte length/digest, archive-policy version, runner OS/architecture, and expanded paths. `complete` contains schema/digest. Complete entries are immutable.

Exact lookup uses only the digest directory. Invalid exact entries are quarantined under lock before fallback. For each requested prefix in order, fallback validates and orders candidate metadata records by `createdAt` descending, then digest ascending, and considers the first 256 valid records in that order; it selects the first candidate whose key has that prefix. It requires matching schema/archive policy/OS/architecture. A fallback remains `cache-hit=false`.

Save uses a same-filesystem private staging directory. It fsyncs payload/metadata/complete/staging, publishes atomically, then fsyncs the parent. Per-entry locks use `O_CREAT|O_EXCL`, bounded backoff/timeout, and a PID/boot-ID/timestamp record. The winner never replaces a complete entry; a loser validates it and returns `raced`. Quarantine uses unique mode-0700 names.

## Archive safety and limits

`tar.zst` is created/read by versioned bundled dependencies, not host tar/pigz or a shell. Traversal is rooted in private staging and never follows symlinks.

Restore verifies digest then rejects absolute/traversal paths, duplicates/conflicts, all links, special files, and non-directory parents. Limits: 2 GiB compressed, 8 GiB extracted, 100,000 entries, 1 GiB/file, 1 MiB metadata, 20-minute restore. Only complete verified staging content is materialized; errors leave workspace unchanged.

Save applies the same no-symlink rule to source/parents, rejects special and hard-linked files, snapshots input paths, and compares source stat before/after streaming. Save enforces the same compressed, extracted, entry-count, per-file, and metadata limits as restore and has a 20-minute timeout. Thus v1 intentionally excludes `node_modules` and other symlink-preserving workloads.

## Errors, releases, and verification

Invalid exact entries follow the quarantine-then-fallback behavior defined under Entry format and lifecycle. Operational errors fail by default. With `fail-on-cache-error=false`, restore warns and returns `cache-match=error`; save warns and returns `cache-save=error`. Disk/inode exhaustion and lock timeout are classified errors. Logs contain stable event names and key digests only: match/failure class, bytes/files, elapsed time, and save result.

The action ships committed Node 24 bundles. Contract changes follow SemVer; schema changes use a new directory version and documented reader/writer coexistence, capacity headroom, deprecation, and retention plan. Releases use protected immutable tags, full SHAs, notes, and bundle checksum/provenance; consumers pin full SHAs.

Before v1 release: unit tests for grammar, matching, metadata, paths; miss/save/exact/fallback/corrupt/concurrent/interrupted integration tests; malicious archive/root/workspace/source-mutation fixtures; lock, ENOSPC/inode, conflict, limit, retention fault tests; successful/failed-workflow save tests; runner compatibility and cold/warm benchmark checks.

## Adoption contract

Consumers mount a repository-exclusive directory and independently validate restored artifacts where needed. Cache immutable/rebuildable outputs and package-manager download stores—not virtual environments, `node_modules`, credentials, database volumes, or deployment artifacts. The first acceptance case is verified extraction artifacts: both cold generation and warm restore execute the same manifest/digest guard.
