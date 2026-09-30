# Nextcore-Core

Public boot configuration, bounded format codecs, APFS Jumpstart extraction,
and ARM64/ARM64e kernel-collection placement. This is an independent repository;
standalone consumers pin an immutable Git revision. The integration project maintains a tracked source snapshot.

The ARM handoff planner validates segment placement, memory and entry ranges,
flat device-tree structure and boot-argument layout. Configuration distinguishes
an authored x86-EFI ARM JIT fixture from a bounded, explicitly unprovisioned
SPTM-prefix diagnostic. An ARM kernel build does not change the physical x86
execution target. Staging validates data placement and preservation; it does not
establish macOS boot or complete SPTM services.

```sh
cargo test --all-targets
cargo check --no-default-features
```

Original restore components and private input analyses are never bundled. Earlier
release metadata remains intact under `release_provenance` in `repository.json`.


`firmware_dt` borrows raw firmware property bytes and identifies unresolved
value templates separately from runtime data. Converting template-bearing input
to a runtime DeviceTree is explicitly rejected; no template bit is stripped to
make an unresolved input appear ready for XNU.

## September 30 engineering snapshot

Current Status: This module is synchronized from one reviewed immutable integration snapshot. Its source revision and exact dependency pins are recorded in `repository.json`; file sizes and SHA-256 digests are recorded in `repository-files.json`. Existing repository history and license notices are preserved.

Target State: Independently reproducible source and module validation. Module tests establish the stated component behavior. macOS 27 boot and usable installed operation, guest Metal, physical installation and device qualification remain unverified.
