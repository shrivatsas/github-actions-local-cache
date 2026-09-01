# Operator and security guide

## Host setup

The runner operator must provide a local POSIX filesystem. For each repository, create a distinct root owned by the runner service user and set mode `0700`:

```console
install -d -m 0700 -o actions-runner -g actions-runner /media/cache/my-repository
```

Configure byte and inode quotas. Keep the root outside `GITHUB_WORKSPACE`, do not expose it to untrusted or unreviewed pull-request code, and drain the runner before retention or repair work. NFS, SMB, shared roots, Windows, macOS, and containers without the mount are outside v1.

### Shared runner host, multiple repositories

Several repositories may use runners on the same host, but each repository needs its
own `cache-dir`, not a shared parent. For example, provision the directories as the
runner user (or use `-o`/`-g` to assign that user when provisioning as root):

```console
install -d -m 0700 -o actions-runner -g actions-runner /srv/cache/repository-a
install -d -m 0700 -o actions-runner -g actions-runner /srv/cache/repository-b
```

The workflows must then use `cache-dir: /srv/cache/repository-a` and
`cache-dir: /srv/cache/repository-b`, respectively. The action atomically claims a
root for its immutable repository ID and fails with `shared-root-detected` if a
different repository has already claimed it or an existing `v1/<other-repository-id>`
namespace is present. That guard catches configuration mistakes; it does not turn a
shared filesystem or OS user into an isolation boundary. Configure quota and retention
per repository, and drain the affected runner before cleanup.

The minimum supported Actions Runner is v2.328.0. GitHub documents `node24` in [action metadata syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/metadata-syntax), and the [Node 24 runner migration notice](https://github.blog/changelog/2025-09-19-deprecation-of-node-20-on-github-actions-runners/) identifies v2.328.0 as supporting Node 24.

## Workload boundary

Good cache candidates are immutable or rebuildable outputs and package-manager download stores. Do not cache credentials, database volumes, deployment artifacts, virtual environments, or `node_modules`. v1 rejects symlinks, hard-linked files, and special files.

Restore never overwrites an existing top-level destination. Both cold generation and warm restore must feed the same manifest, digest, or package-integrity guard.

## Failure handling

Operational errors fail by default. `fail-on-cache-error: "false"` converts an error to a warning and returns the action-specific `error` output. Logs use stable event names, error classes, and SHA-256 key digests; they do not emit raw keys or paths.

Invalid exact entries are moved under a unique `.quarantine-<digest>-<uuid>` name while holding the digest lock. Inspect or remove quarantines only while the runner is drained.

## Release process

Before publishing v1:

1. Assign a release maintainer and protect immutable version tags.
2. Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`.
3. Build static binaries from the locked dependencies on trusted Linux x64 and arm64 builders.
4. Manually dispatch `Promote verified native bundles` with the successful CI run ID. It downloads the two artifacts, runs `scripts/promote-native-bundles.sh`, commits the verified bundles plus `checksums.sha256` to a promotion branch, and opens a PR targeting `main`. Merge that PR manually after review. The script remains available for audited local promotion.
5. Verify each binary with `file`, execute the lifecycle suite natively on each architecture, and record build provenance.
6. Commit the binaries, review the binary diff/checksums, create release notes, and sign a protected immutable tag. Consumers pin the release's full commit SHA.

GitHub currently exposes `ubuntu-24.04` and `ubuntu-24.04-arm` hosted runner labels for native release validation; see [Choosing the runner for a job](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).
