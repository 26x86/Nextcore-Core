# BP33 guest allocation ledger — review contract

Status: root approved the bounded API and both backing constructors before code. 2026-09-09.
Owner: ARM EFI handoff agent. Root owns integration, dependency pins, Git index,
commits and publication. Work will use a new Core worktree based exactly on
`305e66a2847172a455aaa19264998832a9e5712c`; BP32 EFI/runtime worktrees stay frozen.
No original Apple asset is needed for implementation or testing.

## Current state and desired state

Core `runtime_dt::prepare` produces immutable, validated runtime-DT bytes from
explicit source-bound provider values. Its `DeclaredReservation` is a public
coordinate assertion. `write_reserved` validates its extent against a real slice
and copies only after preflight, but does not establish reservation ownership,
non-overlap, provenance of an allocation snapshot, or continuing backing lifetime.

`KcStagingPlan` and `Arm64HandoffLayout` describe placement. `StagedKc` owns verified
bytes in a host Vec; its host allocation is not evidence of guest physical pages.
EFI `ArmPages` owns firmware pages and exposes a checked slice. ISE MemoryService
exclusively borrows that slice with an explicitly configured guest base. These
are useful ownership boundaries, but none maintains guest-purpose reservations.

Add a small no_std Core `guest_memory` module connecting an exclusive backing
owner/loan, an explicitly chosen guest aperture, checked non-overlapping
reservations, and source-bound DT preparation. This is a staging and lifetime
contract. It neither allocates Apple hardware nor proves an Apple handoff ABI.
Existing staging and `runtime_dt` public APIs remain compatible.

## Backing and lifetime

`GuestMemory<'a>` owns a private backing enum with two constructors:

- `from_owned(base, Box<[u8]>) -> Result<GuestMemory<'static>, Error>` consumes a
  fixed-length host allocation. It does not resize it or imply page alignment.
- `from_borrowed(base, &'a mut [u8]) -> Result<GuestMemory<'a>, Error>` consumes an
  exclusive loan. The actual allocator owner, such as EFI ArmPages, cannot be
  freed or accessed through safe Rust until the ledger loan ends. This is an
  explicitly borrowed backing, not a claim that Core frees firmware pages.

The aperture is exactly `[configured guest base, base + observed backing.len())`.
Reject an empty backing and checked-add overflow. There is no independent length
parameter, identity-map assumption, unsafe backing trait, raw-pointer ownership
conversion, resize, rebase, or backing replacement. The two constructors share
all reservation logic. The ledger is neither Clone nor serializable.

A process-local, nonzero AtomicU64 ledger identity is minted once, without wrap or
reuse. Exhaustion is a returned error. Relaxed atomic ordering suffices for
uniqueness. Supported EFI/host targets already have 64-bit atomics (x86_64 and
AArch64); this module will explicitly document that target requirement. Tokens
are not stable storage identifiers or cross-process capabilities. Pointer values
are deliberately not used as identity because allocator addresses can be reused.

## Reservation API

A fixed table holds at most 64 live reservations; this is a documented library
bound, not a claim about the final platform memory-map size. No per-reservation
heap allocation is required. Operations are checked before table/RAM mutation.

- `reserve_at(address, bytes, alignment, purpose)` reserves an exact extent.
- `allocate(bytes, alignment, purpose)` picks the lowest aligned first fit inside
  the aperture, without moving an existing reservation.
- `release(token)` removes one live reservation; bytes are retained unchanged.
- `read(token)` returns a read-only view of that reservation.
- `copy_into(token, offset, bytes)` preflights bounds and copies an already
  prepared payload. RuntimeDeviceTree reservations require the typed DT path.

Allowed purposes are KernelImage, BootArguments, RuntimeDeviceTree, Stack,
TranslationTables, and ProviderData. They are tags for the caller's explicit
contract, not authorization to invent bytes for that purpose. Reject zero length,
non-power-of-two or zero alignment, an unaligned exact address, any checked
arithmetic overflow, an extent outside the aperture, and intersection with any
live reserved capacity. Adjacent ranges are allowed. Empty space is not silently
reserved. Alignment constrains the guest address only, not host page alignment.

Opaque `ReservationToken` has private owner and monotonically unique reservation
serial fields. A live serial is never recycled. A foreign-ledger token, released
token, or token whose slot was reused fails validation before bytes are exposed.
Tokens may be copied; their authority remains conditional on the matching live
ledger entry. There is no token constructor from coordinates.

## Generation and snapshots

The ledger has a non-wrapping monotonically increasing generation. Successful
reservation/release, successful byte copy, successful DT commit, and granting a
mutable execution loan advance it. Failed preflight operations leave generation,
reservations and RAM unchanged. Reserve serials can use the newly committed
monotonic generation, so no second reuse counter is needed.

`snapshot()` provides a borrowed read-only view of the actual aperture and live
records (token, purpose, alignment, address, capacity), plus an opaque copyable
`SnapshotStamp` holding owner/generation. The view prevents simultaneous mutation
through safe Rust; the stamp can later be checked after the view is dropped.
Advancing generation conservatively invalidates all pending DT preparations,
including preparations whose destination reservation was not itself changed.

This generation covers ledger-mediated writes and the granting of a guest RAM
loan. It is not a hardware coherence counter, a DMA detector, or proof that a
caller-supplied property encodes an observed allocation. The backing must remain
exclusively accessed through the owned value/loan contract.

## DT preparation and atomic commit

`bind_device_tree(stamp, destination_token, MaterializedTree)` consumes immutable
output from the existing runtime_dt transformer. It validates current owner and
generation, a live RuntimeDeviceTree-purpose reservation, four-byte guest address
alignment, and full output fit. It returns a non-Clone `PreparedDeviceTree<'src>`
with private tree, owner, generation and destination identity fields. It does not
expose mutable prepared bytes.

The stamp must be the snapshot used to prepare allocation-dependent values. The
library proves that this stamp is current at bind and commit; it cannot prove
semantic provenance of arbitrary `ProvidedValue.value` byte strings. Target
property schemas, actual provider measurements and boot-ABI adapters remain
explicit later integration work. Provider labels are not authentication.

`commit_device_tree(prepared)` consumes the patch, repeats owner/generation,
liveness, purpose and complete range checks, reserves the next generation, then
performs one slice copy and commits that generation. Returned errors cause no RAM
or reservation/generation changes. Bytes outside the exact DT output, including
unused reserved tail, remain unchanged. A stale patch must be rebuilt from a
current snapshot. The caller does not supply a fresh destination coordinate at
commit, so preparation cannot accidentally be redirected to another reservation.

Preparation buffers may allocate through the existing fallible runtime_dt APIs;
ledger bookkeeping itself is bounded and allocation-free after backing is
supplied. No expression evaluation, zero fallback, guessed ABI, arbitrary literal
rewrite or normal-boot readiness method is introduced.

## Execution and existing image staging integration

`with_guest_memory` grants a scoped exclusive mutable RAM loan and explicit guest
base to a closure, advancing generation before the closure can observe RAM. The
closure cannot return a borrowed RAM/service value in safe Rust. While it runs,
reservation methods and release are unavailable because the ledger is exclusively
borrowed. A future authored EFI consumer can construct a MemoryService inside the
closure, run the JIT, and drop the service before the closure returns.

The loan intentionally allows guest execution to write RAM. Reservations are not
runtime page permissions, and a closure returning an error does not roll back its
writes. The conservative generation change remains even when the closure returns
an error. This explicit execution API is separate from atomic staging copies.

The first image integration uses immutable `StagedKc::bytes()` and checked
`copy_into` to a KernelImage reservation. It does not wrap `stage_into` in an
allegedly atomic mutable callback: that producer can write before returning an
error. Boot arguments can similarly be copied after an independent codec succeeds.
No current macOS27 original trace path is changed in this slice.

## Acceptance and evidence

Authored tests exercise both owned and borrowed backing, a nonzero guest aperture,
identity differing from host addresses, exact and first-fit allocation,
adjacency, full-capacity overlap, purpose, foreign/released/reused tokens, slot
capacity, arithmetic/alignment errors, and retained bytes on release. Internal
boundary tests exercise identity and generation exhaustion without a public reset
or forging API.

An independent byte-occupancy reference model checks 2,000 deterministic
reserve/release steps and first-fit choices by scanning individual byte candidates.
Comparisons include every live range; atomic-failure tests also compare complete
RAM. This is a policy test, not a mirror of the implementation's slot search.

DT tests build synthetic flagged properties, obtain a ledger snapshot, supply
explicit authored values, bind and commit, strictly parse readback, and compare
untouched prefix/tail bytes. Negatives cover foreign stamp/target, wrong purpose,
insufficient destination capacity, mutation between snapshot/bind and commit,
release/reallocate, and malformed/missing bindings from the existing transformer.
The complete backing is byte-compared on every returned commit failure.

A synthetic execution-loan test reads the committed DT and writes a marker inside
the borrowed guest view, then validates retained ownership, generation invalidation
and post-loan readback. Compile-fail doctests establish that RAM/service borrows
cannot escape the execution closure or outlive a borrowed allocator owner. Host
Core tests and an x86_64-unknown-uefi no-default-features check must pass.

Evidence identifies the exact Core base and changed source hashes. It does not
claim actual EFI execution: a separate later authored EFI consumer will use
ArmPages -> borrowed ledger -> reserved DT -> scoped MemoryService. The BP32
frozen EFI proof and original-input receipts are not modified or relabeled.

## Remaining platform work

The ledger observes backing length and retains ownership/loan; EFI still chooses
and verifies host allocation attributes and an explicit guest aperture. Target
runtime-DT schemas and real manufacturing/platform values, all unresolved firmware
templates, complete boot memory regions, boot policy/security state and the exact
macOS27 boot/SPTM data ABI remain unproven. Reservation purposes supply no values
for those requirements. This slice establishes a testable prerequisite only.

## Review decision

Root approved both backing constructors, 64 reservations, opaque identities and
generations, and typed DT commit. The execution closure must be higher-ranked and
compile-fail proofs must demonstrate non-escaping loans. No open API decision
blocks this bounded implementation. Actual EFI consumption remains a follow-up.

## Implemented acceptance (2026-09-09)

Implemented in `src/guest_memory.rs`, exported from `src/lib.rs`; existing runtime-DT
and image APIs remain unchanged. `tests/guest_memory.rs` has 10 authored test groups
including the 2,000-step independent oracle, six separate stale-commit causes,
foreign/dropped-owner and same-slot reuse, exact reserved capacity checks, and
full-RAM comparisons after commit rejection. Three private unit tests cover identity
and generation exhaustion, including DT commit failure. Three compile-fail
doctests cover allocator lifetime, escaping a RAM slice, and escaping a service
that contains the loan. The existing authored KC fixture adds one verified immutable
producer -> reservation copy test in `tests/kc_staging.rs`.

From the Core checkout, reproduce with:

```sh
cargo test --no-default-features
cargo test
cargo check --no-default-features --target x86_64-unknown-uefi
```

The recorded run used Rust 1.98.1, a separate persistent target directory, and the
resolved lockfile with `--offline --locked`. No-default-feature tests passed 232
host unit/integration tests plus 3 compile-fail doctests; default-feature tests passed 257 host unit/integration
tests plus the same 3 doctests. The UEFI no_std target check passed. These totals
include existing Core regression tests and are not additive distinct cases.

No actual EFI consumer was executed in this slice. The next bounded consumer can
allocate real ArmPages, borrow its exact slice into this ledger at an explicit
guest base, reserve an authored runtime DT and marker, derive synthetic extent
values from its current snapshot, commit, and run a small authored guest through
a scoped MemoryService. It should independently compare the returned values,
untouched RAM, and stale-patch rejection. That consumer must still choose and
verify the actual firmware allocation attributes; it does not establish the
macOS27 runtime-DT schema or original boot/SPTM ABI.
