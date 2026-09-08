# Nextcore-Core

Public boot configuration, bounded format codecs, APFS Jumpstart extraction,
and ARM64/ARM64e kernel-collection placement. This is an independent repository;
the integration project consumes an immutable submodule commit.

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
