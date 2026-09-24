# Owned framebuffer handoff

Current Status: Core reserves and encodes optional guest framebuffer storage;
validation remains separate from firmware presentation or operating-system boot.
Target State: A caller presents the same owned storage described by boot video,
without overlapping kernel, arguments, DeviceTree or boot stack.

## Contract

The public wire source is Apple OSS XNU `ac9718fb1af618d5ce8678d0dc6e8a58f252216f`,
[arm64/boot.h](https://github.com/apple-oss-distributions/xnu/blob/ac9718fb1af618d5ce8678d0dc6e8a58f252216f/pexpert/pexpert/arm64/boot.h).
Its LP64 boot-video words describe base, display, row bytes, dimensions and depth.
This module independently chooses a physical base, display code 1, depth 32 with
zero rotation/scale parameters, and little-endian XRGB8888 pixels. The header
does not prove target display compatibility or prescribe the allocation policy.

`Arm64HandoffPlan::new` retains its existing behavior and accepts unmanaged video
metadata. `new_with_framebuffer(source, input, geometry)` instead requires zero
input video, nonzero u32 width/height/row_bytes, a four-byte-aligned stride, and
stride at least checked width times four. Conflicting input video is rejected.
`framebuffer()` exposes an optional immutable `Arm64FramebufferLayout` containing
`base_phys`, logical `byte_len`, page-rounded `reserved_bytes` and `geometry`.

The framebuffer begins at the unchanged, page-aligned stack top. Its logical
size includes row padding; its reservation rounds upward to 16 KiB. Checked
arithmetic prevents wrap. The complete reservation is included in occupied_end,
allocation_bytes and encoded topOfKernelData. Existing RAM/DRAM validation also
requires free memory above that occupied boundary. Sequential placement prevents
overlap without accepting a caller-supplied framebuffer address. The backing is
part of the caller-provided staging allocation, never a host GOP pointer.

Staging zeroes all pixels, row padding and page padding and verifies every byte.
`verify` describes the initial staged state, so intentional guest framebuffer
writes invalidate that initial-state check. Source integrity and later display
readback are separate caller responsibilities. Execution readiness stays false.

## Validation

Independent authored fixtures check exact wire bytes and reservation intervals,
legacy equivalence, invalid geometry, conflicting video, insufficient RAM,
address overflow, and corruption in pixels and padding. Host tests and no_std
x86_64 UEFI compilation cover Core only. Root owns actual EFI guest-write/GOP
readback and physical hardware acceptance; neither is implied by Core tests.

Validation on 2026-09-12 used Rust 1.97.1 / Cargo 1.97.1, edition 2021 (no
explicit module MSRV). The final source passed scoped rustfmt, all nine
`arm64_handoff` tests (six new framebuffer cases), all fifteen existing
`xnu_arm64_boot_args` tests, and `cargo check --no-default-features` on both
`x86_64-unknown-linux-gnu` and `x86_64-unknown-uefi`. A first compile caught an
initializer field placed on the codec input instead of the plan; it was fixed
before these passing checks. This is host-test and target-compile evidence only.
