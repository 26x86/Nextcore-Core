# Explicit mapped diagnostic tables

## Current Status

The ordinary trace parsers reject any MemoryProfile field. The mapped parser is
a separate caller capability and accepts only `mapped-normal-nc-v1` with an
explicit software platform. Selection does not establish an original OS ABI.

## Target State

`Arm64Stage1Tables::new(physical_base, virtual_base, memory_size)` constructs
immutable 16KiB stage-1 tables for identity and high virtual aliases of the same
RAM. Inputs are page aligned, nonempty and limited to 1GiB. Identity addresses
fit the low 47-bit region; virtual aliases fit the high 47-bit region without
overflow. Tables occupy a separate guest physical region immediately after RAM,
within 48 physical bits, and are not mapped into either alias. Storage is bounded
to 2MiB and allocation failures are returned. Host Vec alignment is not a hardware
table-base promise: this is owned backing for the software diagnostic walker.

The public Arm 100940_0101_en address-translation contract, sections 4.1/4.2
(pp.13-15), supplies 16KiB descriptor geometry. L1/L2/L3 indices use shifts
36/25/14 and eleven bits each. Descriptors are little-endian 64-bit words;
table entries use type 3, page entries type 3 with AF set, AttrIndx 0, EL1 RW,
and executable permissions. MAIR0=0x44 selects Normal non-cacheable memory.
TCR selects T0SZ=T1SZ=17, TG0=2, TG1=1 and IPS=5. Other TCR controls are zero.
Source: https://documentation-service.arm.com/static/5efa1d23dbdee951c1ccdec5

The caller must keep table backing alive and immutable, reserve its physical
range, configure the matching memory service, and translate the selected entry
contract's addresses explicitly. These tables grant diagnostic EL1 RWX access;
they are not production kernel protection, PAC, platform provisioning or normal
entry readiness. Independent walking tests and EFI consumption own validation.

## Validation

Rust 1.97.1 host tests pass with no default features, including the full Core
suite. Six new integration tests cover every page in both aliases across L1/L2
boundaries, unmapped neighbors and table storage, maximum RAM, canonical/range
errors, all old parser APIs, exact selectors, missing platform and unchanged
budget policy. The independent test reader also checks all descriptor attribute
bits. `cargo check --no-default-features --target x86_64-unknown-uefi` passes.
New files pass rustfmt. Clippy passes with the two pre-existing library lints
`manual_is_multiple_of` and `implicit_saturating_sub` allowed; an unmodified
`-D warnings` run reports those older sites outside this implementation.
Actual canonical memory-service and firmware execution are separate integration
checks; host walking is not proof of physical hardware startup.
