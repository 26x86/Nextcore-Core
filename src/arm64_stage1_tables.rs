//! Owned, immutable dual-alias tables for an explicitly selected software diagnostic.
//! Descriptor contract and limits: `docs/ARM64_STAGE1_TABLES.md`.
use alloc::vec::Vec;

pub const PAGE_SIZE: u64 = 16 * 1024;
pub const MAX_MEMORY_SIZE: u64 = 1024 * 1024 * 1024;
pub const MAX_TABLE_BYTES: usize = 2 * 1024 * 1024;
const LOW_END: u64 = 1 << 47;
const HIGH_START: u64 = 0xffff_8000_0000_0000;
const PA_END: u64 = 1 << 48;
const ADDRESS_MASK: u64 = 0x0000_ffff_ffff_c000;
const TCR: u64 = 17 | (2 << 14) | (17 << 16) | (1 << 30) | (5 << 32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidRange,
    Alignment,
    AddressOverflow,
    TableCapacity,
    Allocation,
    UnmappedAddress,
}

#[derive(Debug)]
pub struct Arm64Stage1Tables {
    bytes: Vec<u8>,
    table_base: u64,
    ram_base: u64,
    virtual_base: u64,
    memory_size: u64,
}

impl Arm64Stage1Tables {
    pub fn new(physical_base: u64, virtual_base: u64, memory_size: u64) -> Result<Self, Error> {
        if (physical_base | virtual_base | memory_size) & (PAGE_SIZE - 1) != 0 {
            return Err(Error::Alignment);
        }
        if memory_size == 0 || memory_size > MAX_MEMORY_SIZE {
            return Err(Error::InvalidRange);
        }
        let table_base = physical_base
            .checked_add(memory_size)
            .ok_or(Error::AddressOverflow)?;
        virtual_base
            .checked_add(memory_size)
            .ok_or(Error::AddressOverflow)?;
        // Separate canonical halves make alias overlap impossible.
        if table_base > LOW_END || virtual_base < HIGH_START {
            return Err(Error::InvalidRange);
        }
        let mut result = Self {
            bytes: Vec::new(),
            table_base,
            ram_base: physical_base,
            virtual_base,
            memory_size,
        };
        result.allocate_table()?;
        result.allocate_table()?;
        for offset in (0..memory_size).step_by(PAGE_SIZE as usize) {
            let pa = physical_base + offset;
            result.map_page(0, pa, pa)?;
            result.map_page(PAGE_SIZE as usize, virtual_base + offset, pa)?;
        }
        Ok(result)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Guest table address, not the address of the host Vec allocation.
    pub fn physical_base(&self) -> u64 {
        self.table_base
    }
    pub fn ttbr0(&self) -> u64 {
        self.table_base
    }
    pub fn ttbr1(&self) -> u64 {
        self.table_base + PAGE_SIZE
    }
    pub fn tcr(&self) -> u64 {
        TCR
    }
    pub fn virtual_address(&self, pa: u64) -> Result<u64, Error> {
        let offset = pa
            .checked_sub(self.ram_base)
            .ok_or(Error::UnmappedAddress)?;
        if offset >= self.memory_size {
            return Err(Error::UnmappedAddress);
        }
        self.virtual_base
            .checked_add(offset)
            .ok_or(Error::AddressOverflow)
    }

    fn allocate_table(&mut self) -> Result<usize, Error> {
        let start = self.bytes.len();
        let end = start
            .checked_add(PAGE_SIZE as usize)
            .ok_or(Error::TableCapacity)?;
        if end > MAX_TABLE_BYTES {
            return Err(Error::TableCapacity);
        }
        if self
            .table_base
            .checked_add(end as u64)
            .is_none_or(|end| end > PA_END)
        {
            return Err(Error::AddressOverflow);
        }
        self.bytes
            .try_reserve_exact(PAGE_SIZE as usize)
            .map_err(|_| Error::Allocation)?;
        self.bytes.resize(end, 0);
        Ok(start)
    }

    fn map_page(&mut self, root: usize, va: u64, pa: u64) -> Result<(), Error> {
        let mut table = root;
        for shift in [36, 25] {
            let slot = table + (((va >> shift) & 0x7ff) as usize) * 8;
            let descriptor = u64::from_le_bytes(self.bytes[slot..slot + 8].try_into().unwrap());
            table = if descriptor == 0 {
                let next = self.allocate_table()?;
                let value = (self.table_base + next as u64) | 3;
                self.bytes[slot..slot + 8].copy_from_slice(&value.to_le_bytes());
                next
            } else {
                ((descriptor & ADDRESS_MASK) - self.table_base) as usize
            };
        }
        let slot = table + (((va >> 14) & 0x7ff) as usize) * 8;
        self.bytes[slot..slot + 8].copy_from_slice(&(pa | 0x403).to_le_bytes());
        Ok(())
    }
}
