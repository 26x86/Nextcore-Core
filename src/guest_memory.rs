//! Exclusive host backing, explicit guest coordinates, and live reservations.
//!
//! This ledger is a staging/lifetime contract, not a hardware memory map or a
//! runtime access-control policy. Purpose tags do not authenticate provider data.
//! The x86_64/AArch64 targets require 64-bit atomics for process-local identities.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::runtime_dt::MaterializedTree;

pub const MAX_RESERVATIONS: usize = 64;

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    EmptyBacking,
    ArithmeticOverflow,
    IdentityExhausted,
    GenerationExhausted,
    InvalidSize,
    InvalidAlignment,
    UnalignedAddress,
    OutsideAperture,
    Overlap,
    ReservationLimit,
    NoSpace,
    ForeignOwner,
    ReleasedReservation,
    StaleSnapshot,
    WrongPurpose,
    DestinationCapacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    KernelImage,
    BootArguments,
    RuntimeDeviceTree,
    Stack,
    TranslationTables,
    ProviderData,
}

/// Observed length in a caller-selected guest coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    address: u64,
    bytes: u64,
}

impl Extent {
    pub fn address(self) -> u64 {
        self.address
    }

    pub fn bytes(self) -> u64 {
        self.bytes
    }

    pub fn end(self) -> u64 {
        // Every Extent is constructed after checking this addition.
        self.address + self.bytes
    }
}

/// An opaque, process-local reference to one live reservation. Not serializable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservationToken {
    owner: u64,
    serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotStamp {
    owner: u64,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservationRecord {
    token: ReservationToken,
    extent: Extent,
    alignment: u64,
    purpose: Purpose,
}

impl ReservationRecord {
    pub fn token(self) -> ReservationToken {
        self.token
    }

    pub fn extent(self) -> Extent {
        self.extent
    }

    pub fn alignment(self) -> u64 {
        self.alignment
    }

    pub fn purpose(self) -> Purpose {
        self.purpose
    }
}

/// Borrows actual live records; a detached stamp must be revalidated before use.
pub struct AllocationSnapshot<'a> {
    aperture: Extent,
    stamp: SnapshotStamp,
    records: &'a [Option<ReservationRecord>; MAX_RESERVATIONS],
}

impl AllocationSnapshot<'_> {
    pub fn aperture(&self) -> Extent {
        self.aperture
    }

    pub fn stamp(&self) -> SnapshotStamp {
        self.stamp
    }

    pub fn generation(&self) -> u64 {
        self.stamp.generation
    }

    pub fn records(&self) -> impl Iterator<Item = ReservationRecord> + '_ {
        self.records.iter().flatten().copied()
    }
}

enum Backing<'a> {
    Owned(Box<[u8]>),
    Borrowed(&'a mut [u8]),
}

impl Backing<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Borrowed(bytes) => bytes,
        }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Borrowed(bytes) => bytes,
        }
    }
}

/// Owns a fixed allocation or an exclusive lifetime-bound loan from its owner.
pub struct GuestMemory<'a> {
    backing: Backing<'a>,
    aperture: Extent,
    owner: u64,
    generation: u64,
    reservations: [Option<ReservationRecord>; MAX_RESERVATIONS],
}

/// Immutable prepared bytes bound to one ledger generation and destination.
#[derive(Debug)]
pub struct PreparedDeviceTree<'source> {
    tree: MaterializedTree<'source>,
    stamp: SnapshotStamp,
    destination: ReservationToken,
}

impl PreparedDeviceTree<'_> {
    pub fn bytes(&self) -> &[u8] {
        self.tree.bytes()
    }

    pub fn source_sha256(&self) -> [u8; 32] {
        self.tree.source_sha256()
    }
}

fn mint_owner(counter: &AtomicU64) -> Result<u64, Error> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .map_err(|_| Error::IdentityExhausted)
}

fn validate_request(bytes: u64, alignment: u64) -> Result<(), Error> {
    if bytes == 0 {
        return Err(Error::InvalidSize);
    }
    if !alignment.is_power_of_two() {
        return Err(Error::InvalidAlignment);
    }
    Ok(())
}

fn align_up(address: u64, alignment: u64) -> Result<u64, Error> {
    address
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or(Error::ArithmeticOverflow)
}

impl GuestMemory<'static> {
    /// Consumes the Box, also on constructor failure. Does not imply page alignment.
    pub fn from_owned(base: u64, bytes: Box<[u8]>) -> Result<Self, Error> {
        Self::new(base, Backing::Owned(bytes))
    }
}

impl<'a> GuestMemory<'a> {
    /// The allocator owner remains borrowed until this ledger is dropped.
    ///
    /// ```compile_fail,E0597
    /// use nextcore_core::guest_memory::GuestMemory;
    /// let ledger = {
    ///     let mut owner = vec![0u8; 64];
    ///     GuestMemory::from_borrowed(0x8000, &mut owner).unwrap()
    /// };
    /// assert_eq!(ledger.aperture().bytes(), 64);
    /// ```
    pub fn from_borrowed(base: u64, bytes: &'a mut [u8]) -> Result<Self, Error> {
        Self::new(base, Backing::Borrowed(bytes))
    }

    fn new(base: u64, backing: Backing<'a>) -> Result<Self, Error> {
        let bytes = u64::try_from(backing.bytes().len()).map_err(|_| Error::ArithmeticOverflow)?;
        if bytes == 0 {
            return Err(Error::EmptyBacking);
        }
        base.checked_add(bytes).ok_or(Error::ArithmeticOverflow)?;
        Ok(Self {
            backing,
            aperture: Extent {
                address: base,
                bytes,
            },
            owner: mint_owner(&NEXT_OWNER)?,
            generation: 0,
            reservations: [None; MAX_RESERVATIONS],
        })
    }

    pub fn aperture(&self) -> Extent {
        self.aperture
    }

    pub fn snapshot(&self) -> AllocationSnapshot<'_> {
        AllocationSnapshot {
            aperture: self.aperture,
            stamp: SnapshotStamp {
                owner: self.owner,
                generation: self.generation,
            },
            records: &self.reservations,
        }
    }

    fn next_generation(&self) -> Result<u64, Error> {
        self.generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)
    }

    fn slot(&self, token: ReservationToken) -> Result<usize, Error> {
        if token.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        self.reservations
            .iter()
            .position(|record| record.is_some_and(|record| record.token == token))
            .ok_or(Error::ReleasedReservation)
    }

    fn record(&self, token: ReservationToken) -> Result<ReservationRecord, Error> {
        Ok(self.reservations[self.slot(token)?].expect("live reservation"))
    }

    fn validate_stamp(&self, stamp: SnapshotStamp) -> Result<(), Error> {
        if stamp.owner != self.owner {
            return Err(Error::ForeignOwner);
        }
        if stamp.generation != self.generation {
            return Err(Error::StaleSnapshot);
        }
        Ok(())
    }

    /// Reserve exactly the supplied guest extent; host addresses are unrelated.
    pub fn reserve_at(
        &mut self,
        address: u64,
        bytes: u64,
        alignment: u64,
        purpose: Purpose,
    ) -> Result<ReservationToken, Error> {
        validate_request(bytes, alignment)?;
        if address % alignment != 0 {
            return Err(Error::UnalignedAddress);
        }
        let end = address
            .checked_add(bytes)
            .ok_or(Error::ArithmeticOverflow)?;
        if address < self.aperture.address || end > self.aperture.end() {
            return Err(Error::OutsideAperture);
        }
        if self
            .reservations
            .iter()
            .flatten()
            .any(|record| address < record.extent.end() && record.extent.address < end)
        {
            return Err(Error::Overlap);
        }
        let slot = self
            .reservations
            .iter()
            .position(Option::is_none)
            .ok_or(Error::ReservationLimit)?;
        let generation = self.next_generation()?;
        let token = ReservationToken {
            owner: self.owner,
            serial: generation,
        };
        self.reservations[slot] = Some(ReservationRecord {
            token,
            extent: Extent { address, bytes },
            alignment,
            purpose,
        });
        self.generation = generation;
        Ok(token)
    }

    /// Lowest aligned first fit. Existing reservations are never relocated.
    pub fn allocate(
        &mut self,
        bytes: u64,
        alignment: u64,
        purpose: Purpose,
    ) -> Result<ReservationToken, Error> {
        validate_request(bytes, alignment)?;
        if self.reservations.iter().all(Option::is_some) {
            return Err(Error::ReservationLimit);
        }
        let mut candidate = align_up(self.aperture.address, alignment)?;
        loop {
            let end = candidate
                .checked_add(bytes)
                .ok_or(Error::ArithmeticOverflow)?;
            if end > self.aperture.end() {
                return Err(Error::NoSpace);
            }
            let blocking_end = self
                .reservations
                .iter()
                .flatten()
                .filter(|record| candidate < record.extent.end() && record.extent.address < end)
                .map(|record| record.extent.end())
                .max();
            match blocking_end {
                Some(next) => candidate = align_up(next, alignment)?,
                None => return self.reserve_at(candidate, bytes, alignment, purpose),
            }
        }
    }

    /// Invalidates the token, leaving the released bytes unchanged.
    pub fn release(&mut self, token: ReservationToken) -> Result<(), Error> {
        let slot = self.slot(token)?;
        let generation = self.next_generation()?;
        self.reservations[slot] = None;
        self.generation = generation;
        Ok(())
    }

    fn range(
        &self,
        record: ReservationRecord,
        offset: u64,
        bytes: usize,
    ) -> Result<core::ops::Range<usize>, Error> {
        let bytes = u64::try_from(bytes).map_err(|_| Error::ArithmeticOverflow)?;
        if offset.checked_add(bytes).ok_or(Error::ArithmeticOverflow)? > record.extent.bytes {
            return Err(Error::DestinationCapacity);
        }
        let start = record
            .extent
            .address
            .checked_sub(self.aperture.address)
            .and_then(|relative| relative.checked_add(offset))
            .ok_or(Error::ArithmeticOverflow)?;
        let end = start.checked_add(bytes).ok_or(Error::ArithmeticOverflow)?;
        if end > self.aperture.bytes {
            return Err(Error::OutsideAperture);
        }
        Ok(
            usize::try_from(start).map_err(|_| Error::ArithmeticOverflow)?
                ..usize::try_from(end).map_err(|_| Error::ArithmeticOverflow)?,
        )
    }

    pub fn read(&self, token: ReservationToken) -> Result<&[u8], Error> {
        let record = self.record(token)?;
        let count = usize::try_from(record.extent.bytes).map_err(|_| Error::ArithmeticOverflow)?;
        Ok(&self.backing.bytes()[self.range(record, 0, count)?])
    }

    /// Atomic checked copy of an independently prepared payload, such as
    /// `StagedKc::bytes()`. This method does not validate an image or boot ABI.
    pub fn copy_into(
        &mut self,
        token: ReservationToken,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let record = self.record(token)?;
        if record.purpose == Purpose::RuntimeDeviceTree {
            return Err(Error::WrongPurpose);
        }
        let range = self.range(record, offset, bytes.len())?;
        let generation = self.next_generation()?;
        self.backing.bytes_mut()[range].copy_from_slice(bytes);
        self.generation = generation;
        Ok(())
    }

    fn dt_range(
        &self,
        token: ReservationToken,
        bytes: usize,
    ) -> Result<core::ops::Range<usize>, Error> {
        let record = self.record(token)?;
        if record.purpose != Purpose::RuntimeDeviceTree {
            return Err(Error::WrongPurpose);
        }
        if record.extent.address % 4 != 0 {
            return Err(Error::UnalignedAddress);
        }
        self.range(record, 0, bytes)
    }

    /// Binds a validated tree to the snapshot used to supply allocation values.
    /// Arbitrary provider byte semantics are still the caller's responsibility.
    pub fn bind_device_tree<'source>(
        &self,
        stamp: SnapshotStamp,
        destination: ReservationToken,
        tree: MaterializedTree<'source>,
    ) -> Result<PreparedDeviceTree<'source>, Error> {
        self.validate_stamp(stamp)?;
        self.dt_range(destination, tree.bytes().len())?;
        Ok(PreparedDeviceTree {
            tree,
            stamp,
            destination,
        })
    }

    /// Single checked copy. All returned errors leave RAM and generation intact.
    pub fn commit_device_tree(&mut self, prepared: PreparedDeviceTree<'_>) -> Result<(), Error> {
        self.validate_stamp(prepared.stamp)?;
        let range = self.dt_range(prepared.destination, prepared.tree.bytes().len())?;
        let generation = self.next_generation()?;
        self.backing.bytes_mut()[range].copy_from_slice(prepared.tree.bytes());
        self.generation = generation;
        Ok(())
    }

    /// Grants an exclusive scoped loan, invalidating prepared snapshots first.
    /// Closure writes are NOT rolled back, including when its return value is Err.
    /// Runtime services must be constructed and dropped inside the closure.
    ///
    /// ```compile_fail
    /// use nextcore_core::guest_memory::GuestMemory;
    /// let mut ledger = GuestMemory::from_owned(0x8000, vec![0; 64].into_boxed_slice()).unwrap();
    /// let escaped = ledger.with_guest_memory(|_, ram| ram).unwrap();
    /// escaped[0] = 1;
    /// ```
    ///
    /// ```compile_fail
    /// use nextcore_core::guest_memory::GuestMemory;
    /// struct Service<'a>(&'a mut [u8]);
    /// let mut ledger = GuestMemory::from_owned(0x8000, vec![0; 64].into_boxed_slice()).unwrap();
    /// let service = ledger.with_guest_memory(|_, ram| Service(ram)).unwrap();
    /// service.0[0] = 1;
    /// ```
    pub fn with_guest_memory<R>(
        &mut self,
        run: impl for<'loan> FnOnce(u64, &'loan mut [u8]) -> R,
    ) -> Result<R, Error> {
        self.generation = self.next_generation()?;
        Ok(run(self.aperture.address, self.backing.bytes_mut()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_exhaustion_never_reuses_or_wraps() {
        let local = AtomicU64::new(u64::MAX - 1);
        assert_eq!(mint_owner(&local), Ok(u64::MAX - 1));
        assert_eq!(mint_owner(&local), Err(Error::IdentityExhausted));
        assert_eq!(mint_owner(&local), Err(Error::IdentityExhausted));
        assert_eq!(local.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn generation_exhaustion_preserves_state_and_does_not_run_closure() {
        let mut data = [0x5a; 64];
        let mut ledger = GuestMemory::from_borrowed(0x1000, &mut data).unwrap();
        let token = ledger.allocate(32, 8, Purpose::KernelImage).unwrap();
        ledger.generation = u64::MAX;
        let before = ledger.reservations;
        assert_eq!(
            ledger.allocate(8, 8, Purpose::Stack),
            Err(Error::GenerationExhausted)
        );
        assert_eq!(ledger.release(token), Err(Error::GenerationExhausted));
        assert_eq!(
            ledger.copy_into(token, 0, &[1]),
            Err(Error::GenerationExhausted)
        );
        assert_eq!(
            ledger.with_guest_memory(|_, _| panic!("must not run")),
            Err(Error::GenerationExhausted)
        );
        assert_eq!(ledger.reservations, before);
        assert_eq!(ledger.generation, u64::MAX);
        assert_eq!(ledger.backing.bytes(), &[0x5a; 64]);
    }

    #[test]
    fn dt_commit_generation_exhaustion_is_atomic() {
        let source = crate::flat_dt::encode(&crate::flat_dt::FlatNode {
            name: "".into(),
            properties: alloc::vec![],
            children: alloc::vec![],
        })
        .unwrap();
        let tree = crate::runtime_dt::prepare(&source, &[], 1024).unwrap();
        let mut bytes = [0xc3; 128];
        let mut ledger = GuestMemory::from_borrowed(0x4000, &mut bytes).unwrap();
        let target = ledger.allocate(128, 4, Purpose::RuntimeDeviceTree).unwrap();
        ledger.generation = u64::MAX;
        let prepared = ledger
            .bind_device_tree(ledger.snapshot().stamp(), target, tree)
            .unwrap();
        let before = ledger.reservations;
        assert_eq!(
            ledger.commit_device_tree(prepared),
            Err(Error::GenerationExhausted)
        );
        assert_eq!(ledger.generation, u64::MAX);
        assert_eq!(ledger.reservations, before);
        assert_eq!(ledger.backing.bytes(), &[0xc3; 128]);
    }
}
