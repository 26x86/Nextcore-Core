# Firmware SHA-256 backend

The runtime DeviceTree transformer introduced SHA-256 with no_std. A real debug
UEFI code-generation build reproduced an LLVM failure in sha2's accelerated x86
backend even though cargo check and release firmware passed. This was exposed by
the existing NXAPFS all-features CI build; that gate stays enabled.

UEFI targets explicitly select sha2's force-soft feature. Hash format, provider
binding and APIs do not change, and host targets retain their normal dependency
configuration. The dependency belongs in Core, where hashing is used. No copied
cryptographic implementation or architecture detection is introduced.

Acceptance includes actual debug and release no_std UEFI code generation, Core
regressions, canonical EFI debug NXAPFS link and actual NXDT serialized hashing
checks. cargo check alone does not cover this compiler failure. Historical failed
CI and local code-generation logs remain evidence; their results are not relabeled.
