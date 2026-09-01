# github-actions-local-cache

Secure, persistent local filesystem caches for Linux self-hosted GitHub Actions runners. The cache engine is written in Rust; dependency-free Node 24 launchers let GitHub invoke the bundled `linux-x64` or `linux-arm64` executable without a shell.

Cache data is an optimization, never a source of authority. Use a repository-exclusive mount and independently verify restored artifacts whenever correctness or provenance matters.

## Usage

Provision one absolute cache directory per repository, owned by the runner user with
mode `0700`, outside `GITHUB_WORKSPACE`. The runner must be Actions Runner v2.328.0
or newer. Do not point multiple repositories at a common parent such as
`/media/cache/actions`; repository-ID namespacing is not an isolation boundary.

```yaml
- id: local-cache
  uses: shrivatsas/github-actions-local-cache/restore@<full-commit-sha>
  with:
    path: fixtures/extraction/demo-seeds/*.json.gz
    key: demo-bootstrap-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('fixtures/**/*.json.gz') }}
    cache-dir: /srv/cache/example-repository
    restore-keys: |
      demo-bootstrap-v1-${{ runner.os }}-${{ runner.arch }}-

# Generate or consume the files, then run the same manifest/digest guard on
# both cold and warm paths.

- if: ${{ success() && steps.local-cache.outputs.cache-match != 'exact' }}
  uses: shrivatsas/github-actions-local-cache/save@<full-commit-sha>
  with:
    path: fixtures/extraction/demo-seeds/*.json.gz
    key: demo-bootstrap-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('fixtures/**/*.json.gz') }}
    cache-dir: /srv/cache/example-repository
```

Restore reports `cache-hit=true` only for an exact key and reports `cache-match` as `exact`, `fallback`, `miss`, or `error`. Save reports `cache-save` as `saved`, `raced`, `skipped-no-paths`, or `error`.

## Documentation

- [Architecture and lifecycle](docs/architecture.md)
- [Operator and security guide](docs/operations.md)
- [Bundle build provenance](docs/build-provenance.md)
- [v1 specification](docs/local-cache-spec.md)
- [Specification review record](docs/spec-review.md)

## Development

```console
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Release commits include both static Linux executables at `restore/dist/local-cache-linux-{x64,arm64}` and `save/dist/local-cache-linux-{x64,arm64}`. See [the release process](docs/operations.md#release-process) before publishing.
