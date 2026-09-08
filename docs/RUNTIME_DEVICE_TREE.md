# Runtime DeviceTree materialization contract

Root owns this isolated Core implementation based on the reviewed BP32 authored
prototype. It is separate from the frozen BP31 conditional-comparison integration
and BP32 native-MMU source. Production ownership stays in this Core repository.

The pure materializer binds explicit provider bytes to SHA-256 of the full source
and the original property offset. It does not evaluate firmware expressions,
replace literals, invent absent values, authenticate a provider label or declare
normal boot readiness. Every flagged property must have exactly one valid source-
bound value. All output is prepared and checked with the existing strict runtime
validator before a single bounded destination copy. Every returned error leaves
source and destination unchanged; unused reservation bytes stay unchanged.

A declared reservation is a caller assertion, not an allocator ownership token.
The actual RAM slice determines backing size; its configured guest base is not
the firmware host PA. Real nonoverlapping allocation ownership and generation are
subsequent staging-layer work. The API does not accept unresolved values as zero.

The existing 1MiB/depth32/1024-node/4096-property/64-per-node runtime profile is
preserved. The audited original tree has more than4096 properties; this change
does not claim it can be used for normal macOS27 startup. Larger runtime limits,
target schemas, live monitor state and boot-argument ABI require separate proof.

Core tests use independently authored bytes and check nested output, resize,
literal/padding preservation, source binding, missing/duplicate/invalid values,
malformed structure, count/length/offset overflow and atomic failures. They read
successful output using an independent byte reader. The earlier isolated
prototype separately read authored data through ISE MemoryService; Core does not
add an ISE dependency or claim that earlier execution was this new module source.
A later EFI consumer must read generated data through the current JIT/provider.

Dependencies: the existing no_std parsers and sha2 without its std feature.
No Apple bytes, private expressions, runtime source copies or new repository are
introduced. Target boot ABI, normal OS boot and guest Metal remain incomplete.
