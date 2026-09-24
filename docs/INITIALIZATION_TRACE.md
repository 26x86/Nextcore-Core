# Explicit initialization diagnostic configuration

Current Status: Core validates a separately selected initialization diagnostic
budget. Validation alone does not execute initialization or establish readiness.
Target State: Give an explicitly built diagnostic enough bounded execution time
to observe the next initialization state or fault while preserving all providers.

`parse_arm64_trace_configuration_with_initialization_tier(input)` accepts exact
`Trace.DiagnosticTier = initialization-67108864` only with instruction budget
67,108,864 and the existing named `nextcore-irq-compat-v1` software platform.
It also accepts existing exact deep-16384 and long-65536 selections. Without a
selector it retains the existing approved tiers through 4096; no platform still
means at most eight instructions.

Default, explicit-limit, deep and long APIs reject the initialization selector,
including with smaller budgets. The explicit-limit API does not accept the new
ceiling as an argument. Unknown values, types, duplicates, mismatched budgets
and missing or invalid named profiles fail. All existing memory, handoff ABI,
platform, path and optional video validation remains intact. No normal readiness
gate, machine state or instruction semantics changes.

The motivation is bounded observation of a large initialization walk, not a
promise that this budget completes a macOS phase. Independent authored parser
matrices compare lower-tier behavior and rejected boundaries. Root owns the EFI
build/selection acknowledgements, execution proof, original-input metadata and
replay. No original input or private structure is needed by this parser.

Validation on 2026-09-12 with Rust/Cargo 1.97.1 passed four initialization matrix
tests, five existing long tests, twelve platform/deep tests and three optional
video tests. `cargo check --no-default-features --target x86_64-unknown-uefi`
also passed. Matrices compare complete configurations, not just accepted counts.
