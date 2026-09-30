//! Opt-in borrowed backing check for an already planned ARM64 handoff.
//! The caller retains firmware page ownership and all entry decisions.
use crate::xnu_arm64_handoff::{Arm64HandoffLayout, ARM_BOOT_STACK_BYTES};

const PAGE: u64 = 16 * 1024;
const PA_LIMIT: u64 = 1 << 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm64BackingError {
    InvalidLayout,
    InvalidRange,
    InsufficientRam,
    OverlappingGuestRanges,
    OverlappingHostRanges,
}

/// Exclusively borrows guest RAM and immutably borrows separate page tables.
/// This does not allocate pages, populate tables, or authorize execution.
pub struct Arm64HandoffBacking<'a> {
    ram: &'a mut [u8],
    ram_base: u64,
    tables: &'a [u8],
    table_base: u64,
    handoff_offset: usize,
    handoff_bytes: usize,
}

impl<'a> Arm64HandoffBacking<'a> {
    pub fn new(
        layout: &Arm64HandoffLayout,
        ram: &'a mut [u8],
        ram_base: u64,
        tables: &'a [u8],
        table_base: u64,
    ) -> Result<Self, Arm64BackingError> {
        use Arm64BackingError as E;
        let kernel_end = layout.kernel_phys.checked_add(layout.kernel_bytes as u64);
        let handoff_end = layout
            .kernel_phys
            .checked_add(layout.allocation_bytes as u64);
        if layout.kernel_bytes == 0
            || layout.kernel_phys % PAGE != 0
            || layout.kernel_bytes % PAGE as usize != 0
            || layout.allocation_bytes % PAGE as usize != 0
            || layout.occupied_end % PAGE != 0
            || layout.allocation_bytes <= layout.kernel_bytes
            || kernel_end.is_none_or(|end| end != layout.boot_args_phys)
            || layout.boot_args_phys.checked_add(PAGE) != Some(layout.device_tree_phys)
            || layout.stack_top_phys % PAGE != 0
            || layout
                .device_tree_phys
                .checked_add(PAGE + ARM_BOOT_STACK_BYTES)
                .is_none_or(|minimum| layout.stack_top_phys < minimum)
            || layout.stack_top_phys > layout.occupied_end
            || handoff_end != Some(layout.occupied_end)
        {
            return Err(E::InvalidLayout);
        }
        let ram_len = u64::try_from(ram.len()).map_err(|_| E::InvalidRange)?;
        let table_len = u64::try_from(tables.len()).map_err(|_| E::InvalidRange)?;
        if ram_base % PAGE != 0
            || table_base % PAGE != 0
            || ram_len < PAGE
            || table_len < PAGE
            || ram_len % PAGE != 0
            || table_len % PAGE != 0
            || ram_base >= PA_LIMIT
            || table_base >= PA_LIMIT
            || ram_len > PA_LIMIT - ram_base
            || table_len > PA_LIMIT - table_base
        {
            return Err(E::InvalidRange);
        }
        if ram_base < table_base + table_len && table_base < ram_base + ram_len {
            return Err(E::OverlappingGuestRanges);
        }
        let offset = layout
            .kernel_phys
            .checked_sub(ram_base)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(E::InsufficientRam)?;
        if offset % PAGE as usize != 0
            || offset
                .checked_add(layout.allocation_bytes)
                .is_none_or(|end| end > ram.len())
        {
            return Err(E::InsufficientRam);
        }
        let ram_start = ram.as_ptr() as usize;
        let table_start = tables.as_ptr() as usize;
        let ram_end = ram_start.checked_add(ram.len()).ok_or(E::InvalidRange)?;
        let table_end = table_start
            .checked_add(tables.len())
            .ok_or(E::InvalidRange)?;
        if ram_start < table_end && table_start < ram_end {
            return Err(E::OverlappingHostRanges);
        }
        Ok(Self {
            ram,
            ram_base,
            tables,
            table_base,
            handoff_offset: offset,
            handoff_bytes: layout.allocation_bytes,
        })
    }

    pub fn handoff_mut(&mut self) -> &mut [u8] {
        let end = self.handoff_offset + self.handoff_bytes;
        &mut self.ram[self.handoff_offset..end]
    }

    pub fn service_parts(&mut self) -> (&mut [u8], u64, &[u8], u64) {
        (self.ram, self.ram_base, self.tables, self.table_base)
    }
}
