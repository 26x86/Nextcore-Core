# Explicit long diagnostic configuration

Current Status: Core validates a separate opt-in 65,536-instruction diagnostic
configuration; this is not execution evidence or normal startup readiness.
Target State: A separately built EFI consumer acknowledges the capability and
selection, executes the bounded authored test, and preserves all provider gates.

`parse_arm64_trace_configuration_with_long_tier(input)` accepts the exact string
`Trace.DiagnosticTier = long-65536` only with `InstructionBudget = 65536` and the
existing named `nextcore-irq-compat-v1` software platform. It also accepts the
existing exact `deep-16384` selection with budget 16384. Missing selection retains
the 4096 ceiling and existing approved tiers; the unprofiled ceiling remains 8.

Default, explicit-limit and deep parsers reject every long selection, even when
paired with a small budget. Explicit-limit API arguments remain restricted to
64, 256, 1024 and 4096. Unknown, incorrectly typed and duplicate selectors fail;
selection/budget mismatches and absent or invalid profiles fail. Placement,
handoff, platform, optional video and argument validation remain unchanged.

Tests compare unselected results against existing parsers and cross capability,
selection and budget boundaries, including malformed values and retained platform
checks. Host parser tests and no_std UEFI compilation prove Core only. Root owns
actual EFI budget/old-build rejection and original-input observation. A larger
retired count is not evidence of forward boot progress or physical display.

Validation on 2026-09-12 with Rust/Cargo 1.97.1 passed five new long-selection
tests, twelve existing platform/deep tests, three existing video-selection tests,
and `cargo check --no-default-features --target x86_64-unknown-uefi`. The new tests
include capability/selector/budget matrices and compare complete parsed results
with lower-tier results after changing only the expected instruction budget.
