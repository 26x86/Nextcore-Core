//! Physical placement/encoding for the explicit ARM64 UEFI staging path.
//! This prepares bytes; it supplies no trust, CPU, device or execution approval.
use crate::{
    kc_staging::{KcStagingError, KcStagingPlan, StagingVerification},
    xnu_arm64_boot_args::{
        encode_arm64_boot_args, Arm64BootArgsEncoding, Arm64BootArgsError, Arm64BootArgsInput,
        Arm64BootVideo, VMAPPLE_PAGE_SIZE,
    },
};
use core::fmt;

pub const MAX_DEVICE_TREE_BYTES: usize = 1024 * 1024;
pub const ARM_BOOT_STACK_BYTES: u64 = 64 * 1024;
pub const ARM_BOOTSTRAP_BLOCK_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Arm64PlacementInput<'a> {
    pub physical_base: u64,
    pub virtual_base: u64,
    pub memory_size: u64,
    pub actual_memory_size: u64,
    pub kernel_phys: u64,
    pub device_tree: &'a [u8],
    pub command_line: &'a str,
    pub machine_type: u32,
    pub boot_flags: u64,
    pub video: Arm64BootVideo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arm64HandoffLayout {
    pub kernel_phys: u64,
    pub kernel_bytes: usize,
    pub collection_header_phys: u64,
    pub entry_phys: u64,
    pub boot_args_phys: u64,
    pub device_tree_phys: u64,
    pub stack_top_phys: u64,
    pub occupied_end: u64,
    pub allocation_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arm64HandoffError {
    Staging(KcStagingError),
    BootArgs(Arm64BootArgsError),
    InvalidPlacement,
    BootstrapCorrespondence,
    DeviceTreeTooLarge,
    AddressOverflow,
    DestinationSize,
    ReadbackMismatch,
}
impl fmt::Display for Arm64HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ARM64_HANDOFF_{self:?}")
    }
}
impl core::error::Error for Arm64HandoffError {}
type Result<T> = core::result::Result<T, Arm64HandoffError>;
use Arm64HandoffError as E;

/// Source and DT remain borrowed; a caller cannot substitute data after plan validation.
pub struct Arm64HandoffPlan<'a> {
    staging: KcStagingPlan<'a>,
    layout: Arm64HandoffLayout,
    boot_args: Arm64BootArgsEncoding,
    device_tree: &'a [u8],
}
impl<'a> Arm64HandoffPlan<'a> {
    pub fn new(source: &'a [u8], input: Arm64PlacementInput<'a>) -> Result<Self> {
        if input.device_tree.is_empty() || input.device_tree.len() > MAX_DEVICE_TREE_BYTES {
            return Err(E::DeviceTreeTooLarge);
        }
        let staging = KcStagingPlan::new_arm64(source).map_err(E::Staging)?;
        if input.kernel_phys % VMAPPLE_PAGE_SIZE != 0 || input.kernel_phys < input.physical_base {
            return Err(E::InvalidPlacement);
        }
        let physical_offset = input
            .kernel_phys
            .checked_sub(input.physical_base)
            .ok_or(E::AddressOverflow)?;
        let expected_virtual = input
            .virtual_base
            .checked_add(physical_offset)
            .ok_or(E::AddressOverflow)?;
        // A single linear VA/PA correspondence includes the within-32MiB block
        // offset needed by the public bootstrap mapping. This is not a proof of
        // target bootstrap-table capacity or target-specific header placement.
        if expected_virtual != staging.minimum_virtual_address()
            || input.virtual_base % ARM_BOOTSTRAP_BLOCK_BYTES
                != input.physical_base % ARM_BOOTSTRAP_BLOCK_BYTES
        {
            return Err(E::BootstrapCorrespondence);
        }
        let kernel_bytes = staging.arena_size();
        let boot_args_phys = input
            .kernel_phys
            .checked_add(kernel_bytes as u64)
            .ok_or(E::AddressOverflow)?;
        let device_tree_phys = boot_args_phys
            .checked_add(VMAPPLE_PAGE_SIZE)
            .ok_or(E::AddressOverflow)?;
        let stack_base = align_up(
            device_tree_phys
                .checked_add(input.device_tree.len() as u64)
                .ok_or(E::AddressOverflow)?,
        )?;
        let occupied_end = stack_base
            .checked_add(ARM_BOOT_STACK_BYTES)
            .ok_or(E::AddressOverflow)?;
        let allocation_bytes = usize::try_from(
            occupied_end
                .checked_sub(input.kernel_phys)
                .ok_or(E::AddressOverflow)?,
        )
        .map_err(|_| E::AddressOverflow)?;
        if allocation_bytes > isize::MAX as usize {
            return Err(E::AddressOverflow);
        }
        let layout = Arm64HandoffLayout {
            kernel_phys: input.kernel_phys,
            kernel_bytes,
            collection_header_phys: input
                .kernel_phys
                .checked_add(staging.collection_header_offset() as u64)
                .ok_or(E::AddressOverflow)?,
            entry_phys: input
                .kernel_phys
                .checked_add(staging.outer_entry_offset() as u64)
                .ok_or(E::AddressOverflow)?,
            boot_args_phys,
            device_tree_phys,
            stack_top_phys: occupied_end,
            occupied_end,
            allocation_bytes,
        };
        let boot_args = encode_arm64_boot_args(&Arm64BootArgsInput {
            physical_base: input.physical_base,
            virtual_base: input.virtual_base,
            memory_size: input.memory_size,
            actual_memory_size: input.actual_memory_size,
            top_of_kernel_data: occupied_end,
            kernel_phys: input.kernel_phys,
            kernel_size: kernel_bytes as u64,
            boot_args_phys,
            device_tree_phys,
            device_tree: input.device_tree,
            command_line: input.command_line,
            machine_type: input.machine_type,
            boot_flags: input.boot_flags,
            video: input.video,
        })
        .map_err(E::BootArgs)?;
        Ok(Self {
            staging,
            layout,
            boot_args,
            device_tree: input.device_tree,
        })
    }
    pub fn layout(&self) -> &Arm64HandoffLayout {
        &self.layout
    }
    pub fn staging(&self) -> &KcStagingPlan<'a> {
        &self.staging
    }
    pub fn boot_args(&self) -> &Arm64BootArgsEncoding {
        &self.boot_args
    }
    pub const fn execution_ready(&self) -> bool {
        false
    }

    /// Initialize and fully read back the allocation. No pointers or PAC words
    /// are relocated: KC fixup ownership remains with its target consumer.
    pub fn stage_into(&self, destination: &mut [u8]) -> Result<StagingVerification> {
        if destination.len() != self.layout.allocation_bytes {
            return Err(E::DestinationSize);
        }
        destination.fill(0);
        let verification = self
            .staging
            .stage_into(&mut destination[..self.layout.kernel_bytes])
            .map_err(E::Staging)?;
        let args = (self.layout.boot_args_phys - self.layout.kernel_phys) as usize;
        let dt = (self.layout.device_tree_phys - self.layout.kernel_phys) as usize;
        destination[args..args + self.boot_args.as_bytes().len()]
            .copy_from_slice(self.boot_args.as_bytes());
        destination[dt..dt + self.device_tree.len()].copy_from_slice(self.device_tree);
        self.verify(destination)?;
        Ok(verification)
    }

    pub fn verify(&self, destination: &[u8]) -> Result<()> {
        if destination.len() != self.layout.allocation_bytes {
            return Err(E::DestinationSize);
        }
        self.staging
            .verify(&destination[..self.layout.kernel_bytes])
            .map_err(E::Staging)?;
        let args = (self.layout.boot_args_phys - self.layout.kernel_phys) as usize;
        let dt = (self.layout.device_tree_phys - self.layout.kernel_phys) as usize;
        for (offset, byte) in destination
            .iter()
            .enumerate()
            .skip(self.layout.kernel_bytes)
        {
            let expected = if offset >= args && offset - args < self.boot_args.as_bytes().len() {
                self.boot_args.as_bytes()[offset - args]
            } else if offset >= dt && offset - dt < self.device_tree.len() {
                self.device_tree[offset - dt]
            } else {
                0
            };
            // SAFETY: live initialized byte; volatile prevents store forwarding
            // by the compiler from replacing actual staging-memory readback.
            if unsafe { core::ptr::read_volatile(byte) } != expected {
                return Err(E::ReadbackMismatch);
            }
        }
        Ok(())
    }
}
fn align_up(value: u64) -> Result<u64> {
    Ok(value
        .checked_add(VMAPPLE_PAGE_SIZE - 1)
        .ok_or(E::AddressOverflow)?
        & !(VMAPPLE_PAGE_SIZE - 1))
}
