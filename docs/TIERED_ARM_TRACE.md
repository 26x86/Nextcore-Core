# Explicit tiered ARM diagnostic parser

The existing `parse_arm64_trace_configuration` entry keeps its profile maximum
of 64 instructions and absent-profile maximum of 8. The new
`parse_arm64_trace_configuration_with_limit` entry requires an explicit limit
of 64, 256, 1024 or 4096. Limits 0, 65 and 4097 are invalid. Budgets above 64
must be one of 256, 1024 or 4096 and fit within that limit. Selecting a larger
limit never provisions a missing platform or raises the absent-profile
maximum of 8. Memory, handoff, platform and DT configuration checks still use
the same parser implementation.

This entry is for an explicit diagnostic-only EFI feature. Default callers
continue using the original function. Original bytes remain unchanged; a
budget outcome proves only bounded instruction execution, not normal boot.
