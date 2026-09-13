# Opt-in trace framebuffer request

Current Status: The trace configuration accepts an explicit framebuffer request. The selector alone does not create guest memory, query GOP, populate boot video arguments, or prove presentation.

Target State: The owning EFI consumer queries an actual GOP mode, reserves guest framebuffer memory with the guest-memory owner, and reports configuration and presentation evidence separately from bounded execution.

`Nextcore.Kernel.Trace.Video` is optional. Omission produces `None` and retains all existing behavior. The sole accepted value is the string `gop-framebuffer`, represented as `Some(Arm64TraceVideo::GopFramebuffer)`. Unknown values, non-string values and duplicate keys reject. No instruction-budget, diagnostic-tier, platform, memory-placement or incomplete-handoff requirement changes.

Host tooling must record whether video was requested. A requested selector cannot be treated as honored by an older EFI without an explicit video-ready or video-unavailable marker. Execution completion remains distinct from video configuration and presentation. No such marker establishes installed macOS boot, a physical desktop, or graphics acceleration.

The parent diagnostic CLI accepts `--gop-framebuffer` and writes the selector only when requested. The consumer reports:

```text
NXARMJIT: TRACE_VIDEO_READY width=640 height=480 base=0x42000000 row_bytes=2560 bytes=1228800
NXARMJIT: TRACE_VIDEO_PRESENTED status=SUCCESS
```

The values above are authored examples, not a hardware default. `bytes` is optional. Readiness must describe positive geometry, at least four bytes per visible pixel, a four-byte-aligned row, and a span inside the configured guest RAM. Exactly one ready marker must precede exactly one successful presentation marker. Missing, malformed, duplicated, contradictory or unavailable markers cannot validate video. A consumer failure can report `NXARMJIT: TRACE_VIDEO_UNAVAILABLE error=TOKEN` while continuing headless execution where possible.

The receipt retains `diagnostic_completed` for the existing execution checks, adds `video_requested` and a separate `video` result, and uses `requested_checks_completed` for the CLI exit status. A requested but unvalidated framebuffer therefore exits unsuccessfully without erasing an independently completed execution result. Readback/hash markers, when emitted by a separately enabled consumer, are retained in the raw marker list but are not inferred from a successful presentation status.

Validation covers omitted versus selected configuration equality, selector types/values/duplicates, unchanged budget and placement constraints, geometry/marker rejection, and five authored host CLI receipts (headless, ignored selector, ready-only, ready-and-presented, unavailable). Host receipt tests mock QEMU; they prove parsing/configuration/exit behavior only. The parent script requires Python 3.11 or later for its existing `hashlib.file_digest` use.
