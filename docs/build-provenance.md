# Bundle build provenance

The committed v1 development bundles were cross-compiled on 2026-08-28 from `Cargo.lock` using:

- Rust 1.96.1 (`aarch64-apple-darwin` host)
- cargo-zigbuild 0.23.3
- Zig 0.15.2
- targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`
- release profile with LTO, one codegen unit, aborting panics, and stripped symbols

Both outputs are statically linked ELF executables. [checksums.sha256](../checksums.sha256) records the committed bundle digests. These local cross-builds establish package completeness; release publication still requires the native x64/arm64 CI validation, provenance, and maintainer approval described in the [release process](operations.md#release-process).
