# Explicit 16,384-instruction diagnostic tier

BP35 adds `parse_arm64_trace_configuration_with_deep_tier` as a separate opt-in API. Existing default parsing remains bounded to64 with the named software IRQ profile or8 without it. Existing `with_limit` accepts only64/256/1024/4096 and still rejects16384 as a caller ceiling.

The new entry preserves old4096-tier behavior when `Trace.DiagnosticTier` is absent. An explicitly present selector must be the string `deep-16384`, must accompany budget16384, and must select the existing `nextcore-irq-compat-v1` profile. All other selector types/values and mismatched budgets reject. Existing parser entry points reject the deep-only selector, including when a lower budget is supplied. This is a recognized capability selector, not a general budget escape.

HandoffAbi remains `unprovisioned-sptm-prefix`. The existing placement, memory, DT-path and platform configuration validators are reused without changing their accepted constraints. No new field changes Arm64TraceConfiguration's layout or the normal kernel picker. EFI uses this API only behind a separate deep diagnostic feature and reports both build capability and actual deep selection.

Tests preserve the old acceptance matrix, reject arbitrary larger budgets/ceilings, enforce exact selector type/value/budget/profile, reject duplicated selectors, and retain placement/handoff/platform rejection. Actual authored x86 EFI proves16384 retirements/fetches before any original diagnostic is run. A budget result remains a budget result and never establishes OS startup or platform-provider completeness.
