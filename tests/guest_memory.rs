//! Authored backing and wire fixtures; no original platform data or ABI values.
use nextcore_core::{
    flat_dt,
    guest_memory::{Error, GuestMemory, Purpose, ReservationToken, MAX_RESERVATIONS},
    runtime_dt::{self, MaterializedTree, ProvidedValue},
};

fn source_tree() -> Vec<u8> {
    fn property(bytes: &mut Vec<u8>, name: &str, value: &[u8], flag: bool) {
        let mut key = [0; 32];
        key[..name.len()].copy_from_slice(name.as_bytes());
        bytes.extend(key);
        bytes.extend(((value.len() as u32) | if flag { 1 << 31 } else { 0 }).to_le_bytes());
        bytes.extend(value);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
    let mut bytes = Vec::from([2u32.to_le_bytes(), 0u32.to_le_bytes()].concat());
    property(&mut bytes, "name", b"\0", false);
    property(
        &mut bytes,
        "authored-aperture",
        b"not-evaluated/guest_extent()\0",
        true,
    );
    bytes
}

fn tree(source: &[u8], address: u64, length: u64) -> MaterializedTree<'_> {
    let id = runtime_dt::template_ids(source).unwrap()[0];
    let value = [address.to_le_bytes(), length.to_le_bytes()].concat();
    runtime_dt::prepare(
        source,
        &[ProvidedValue {
            property: id,
            provider: "authored/observed-extent-v1",
            value: &value,
        }],
        runtime_dt::MAX_OUTPUT_BYTES,
    )
    .unwrap()
}

fn state(ledger: &GuestMemory<'_>) -> (u64, Vec<(u64, u64, Purpose, ReservationToken, Vec<u8>)>) {
    let snapshot = ledger.snapshot();
    (
        snapshot.generation(),
        snapshot
            .records()
            .map(|r| {
                (
                    r.extent().address(),
                    r.extent().bytes(),
                    r.purpose(),
                    r.token(),
                    ledger.read(r.token()).unwrap().to_vec(),
                )
            })
            .collect(),
    )
}

#[test]
fn owned_and_exclusively_borrowed_backing_have_explicit_guest_extent() {
    let mut owner = vec![0xa5; 1024];
    let host_address = owner.as_ptr() as usize as u64;
    {
        let mut ledger = GuestMemory::from_borrowed(0x8000_0000, &mut owner).unwrap();
        assert_ne!(host_address, ledger.aperture().address());
        assert_eq!(ledger.aperture().bytes(), 1024);
        let reservation = ledger.allocate(32, 16, Purpose::BootArguments).unwrap();
        ledger.copy_into(reservation, 4, &[1, 2, 3]).unwrap();
        assert_eq!(&ledger.read(reservation).unwrap()[4..7], &[1, 2, 3]);
    }
    let mut expected = vec![0xa5; 1024];
    expected[4..7].copy_from_slice(&[1, 2, 3]);
    assert_eq!(owner, expected);

    let mut owned = GuestMemory::from_owned(0x4000, owner.into_boxed_slice()).unwrap();
    let all = owned.allocate(1024, 1, Purpose::ProviderData).unwrap();
    assert_eq!(owned.read(all).unwrap(), expected);
    assert_eq!(owned.aperture().end(), 0x4400);
    assert!(matches!(
        GuestMemory::from_owned(0, Box::new([])),
        Err(Error::EmptyBacking)
    ));
    assert!(matches!(
        GuestMemory::from_owned(u64::MAX, Box::new([1])),
        Err(Error::ArithmeticOverflow)
    ));
}

#[test]
fn exact_alignment_adjacency_capacity_and_error_atomicity() {
    let mut bytes = [0x3c; 512];
    {
        let mut ledger = GuestMemory::from_borrowed(0x4000, &mut bytes).unwrap();
        let first = ledger.reserve_at(0x4080, 32, 16, Purpose::Stack).unwrap();
        ledger
            .reserve_at(0x40a0, 32, 16, Purpose::TranslationTables)
            .unwrap();
        let before = state(&ledger);
        for (address, size, align, error) in [
            (0x4000, 0, 1, Error::InvalidSize),
            (0x4000, 8, 0, Error::InvalidAlignment),
            (0x4000, 8, 3, Error::InvalidAlignment),
            (0x4001, 8, 8, Error::UnalignedAddress),
            (0x3fff, 1, 1, Error::OutsideAperture),
            (0x4200, 1, 1, Error::OutsideAperture),
            (0x41ff, 2, 1, Error::OutsideAperture),
            (u64::MAX, 2, 1, Error::ArithmeticOverflow),
            (0x409f, 2, 1, Error::Overlap),
            (0x407f, 2, 1, Error::Overlap),
            (0x4080, 32, 16, Error::Overlap),
            (0x4000, 512, 1, Error::Overlap),
        ] {
            assert_eq!(
                ledger.reserve_at(address, size, align, Purpose::ProviderData),
                Err(error)
            );
            assert_eq!(state(&ledger), before);
        }
        assert_eq!(
            ledger.copy_into(first, 31, &[1, 2]),
            Err(Error::DestinationCapacity)
        );
        assert_eq!(
            ledger.copy_into(first, u64::MAX, &[1]),
            Err(Error::ArithmeticOverflow)
        );
        assert_eq!(state(&ledger), before);
    }
    assert_eq!(bytes, [0x3c; 512]);
}

#[test]
fn foreign_released_and_reused_tokens_are_rejected() {
    let mut a = GuestMemory::from_owned(0x8000, vec![0x41; 128].into_boxed_slice()).unwrap();
    let mut b = GuestMemory::from_owned(0x8000, vec![0x42; 128].into_boxed_slice()).unwrap();
    let old = a.allocate(32, 8, Purpose::KernelImage).unwrap();
    let other = b.allocate(32, 8, Purpose::KernelImage).unwrap();
    assert_ne!(old, other);
    assert_eq!(b.read(old), Err(Error::ForeignOwner));
    assert_eq!(b.release(old), Err(Error::ForeignOwner));
    assert_eq!(a.copy_into(other, 0, &[0]), Err(Error::ForeignOwner));
    a.copy_into(old, 0, &[9]).unwrap();
    a.release(old).unwrap();
    let new = a.allocate(32, 8, Purpose::KernelImage).unwrap();
    assert_ne!(old, new);
    assert_eq!(a.read(old), Err(Error::ReleasedReservation));
    assert_eq!(a.release(old), Err(Error::ReleasedReservation));
    assert_eq!(a.copy_into(old, 0, &[0]), Err(Error::ReleasedReservation));
    assert_eq!(a.read(new).unwrap()[0], 9);
    drop(a);
    let mut c = GuestMemory::from_owned(0x8000, vec![0x41; 128].into_boxed_slice()).unwrap();
    c.allocate(32, 8, Purpose::KernelImage).unwrap();
    assert_eq!(c.read(new), Err(Error::ForeignOwner));
}

#[test]
fn fixed_slots_and_first_fit_do_not_lose_reusable_holes() {
    let mut ledger = GuestMemory::from_owned(0x1001, vec![7; 4096].into_boxed_slice()).unwrap();
    let mut tokens = Vec::new();
    for index in 0..MAX_RESERVATIONS {
        let token = ledger.allocate(16, 16, Purpose::ProviderData).unwrap();
        let record = ledger
            .snapshot()
            .records()
            .find(|r| r.token() == token)
            .unwrap();
        assert_eq!(record.extent().address(), 0x1010 + index as u64 * 16);
        tokens.push(token);
    }
    let before = state(&ledger);
    assert_eq!(
        ledger.allocate(16, 16, Purpose::Stack),
        Err(Error::ReservationLimit)
    );
    assert_eq!(state(&ledger), before);
    ledger.release(tokens[7]).unwrap();
    let replacement = ledger.allocate(16, 16, Purpose::Stack).unwrap();
    assert_eq!(
        ledger
            .snapshot()
            .records()
            .find(|r| r.token() == replacement)
            .unwrap()
            .extent()
            .address(),
        0x1080
    );

    let mut high = GuestMemory::from_owned(u64::MAX - 15, vec![0; 15].into_boxed_slice()).unwrap();
    assert_eq!(
        high.allocate(1, 32, Purpose::Stack),
        Err(Error::ArithmeticOverflow)
    );
    assert_eq!(
        high.allocate(16, 1, Purpose::Stack),
        Err(Error::ArithmeticOverflow)
    );
    let mut small = GuestMemory::from_owned(0x1001, vec![0; 15].into_boxed_slice()).unwrap();
    assert_eq!(small.allocate(1, 16, Purpose::Stack), Err(Error::NoSpace));
    assert_eq!(small.allocate(16, 1, Purpose::Stack), Err(Error::NoSpace));
}

#[test]
fn deterministic_reservations_match_independent_byte_occupancy_oracle() {
    const N: usize = 257;
    const BASE: u64 = 0x1003;
    let mut bytes = [0x5a; N];
    {
        let mut ledger = GuestMemory::from_borrowed(BASE, &mut bytes).unwrap();
        let mut occupied = [false; N];
        let mut model: Vec<(usize, usize, ReservationToken)> = Vec::new();
        let mut random = 0x369c_2468_1357u64;
        for step in 0..2000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            if step % 3 == 0 && !model.is_empty() {
                let index = random as usize % model.len();
                let (start, size, token) = model.remove(index);
                ledger.release(token).unwrap();
                occupied[start..start + size].fill(false);
            } else {
                let size = ((random >> 8) % 31 + 1) as usize;
                let alignment = 1u64 << ((random >> 16) % 6);
                // Independent policy oracle scans individual byte candidates.
                let expected = (0..=N - size).find(|&start| {
                    (BASE + start as u64) % alignment == 0
                        && occupied[start..start + size].iter().all(|&used| !used)
                });
                let before = state(&ledger);
                match (
                    expected,
                    ledger.allocate(size as u64, alignment, Purpose::ProviderData),
                ) {
                    (Some(start), Ok(token)) => {
                        occupied[start..start + size].fill(true);
                        model.push((start, size, token));
                    }
                    (None, Err(Error::NoSpace)) => assert_eq!(state(&ledger), before),
                    pair => panic!("step {step}: policy mismatch {pair:?}"),
                }
            }
            let mut observed: Vec<_> = ledger
                .snapshot()
                .records()
                .map(|r| {
                    (
                        (r.extent().address() - BASE) as usize,
                        r.extent().bytes() as usize,
                        r.token(),
                    )
                })
                .collect();
            observed.sort_by_key(|r| r.0);
            let mut expected = model.clone();
            expected.sort_by_key(|r| r.0);
            assert_eq!(observed, expected, "step {step}");
        }
    }
    assert_eq!(bytes, [0x5a; N]);
}

#[test]
fn dt_commit_uses_observed_snapshot_and_preserves_entire_reserved_tail() {
    let source = source_tree();
    let source_before = source.clone();
    let mut backing = [0xa5; 1024];
    let expected_tree;
    {
        let mut ledger = GuestMemory::from_borrowed(0x8000_0000, &mut backing).unwrap();
        let target = ledger
            .reserve_at(0x8000_0080, 512, 16, Purpose::RuntimeDeviceTree)
            .unwrap();
        let snapshot = ledger.snapshot();
        let materialized = tree(
            &source,
            snapshot.aperture().address(),
            snapshot.aperture().bytes(),
        );
        expected_tree = materialized.bytes().to_vec();
        let prepared = ledger
            .bind_device_tree(snapshot.stamp(), target, materialized)
            .unwrap();
        assert_eq!(
            prepared.source_sha256(),
            runtime_dt::source_identity(&source)
        );
        assert_eq!(prepared.bytes(), expected_tree);
        let old_generation = snapshot.generation();
        ledger.commit_device_tree(prepared).unwrap();
        assert_eq!(ledger.snapshot().generation(), old_generation + 1);
        let readback = &ledger.read(target).unwrap()[..expected_tree.len()];
        flat_dt::validate(readback).unwrap();
        assert_eq!(readback, expected_tree);
        assert_eq!(ledger.copy_into(target, 0, &[0]), Err(Error::WrongPurpose));
    }
    let mut expected = [0xa5; 1024];
    expected[128..128 + expected_tree.len()].copy_from_slice(&expected_tree);
    assert_eq!(backing, expected);
    assert_eq!(source, source_before);
}

#[test]
fn dt_binding_checks_snapshot_target_purpose_capacity_and_alignment() {
    let source = source_tree();
    let mut backing = [0x19; 1024];
    {
        let mut ledger = GuestMemory::from_borrowed(0x8000, &mut backing).unwrap();
        let wrong = ledger.allocate(256, 4, Purpose::ProviderData).unwrap();
        let small = ledger.allocate(4, 4, Purpose::RuntimeDeviceTree).unwrap();
        let unaligned = ledger
            .reserve_at(0x8201, 256, 1, Purpose::RuntimeDeviceTree)
            .unwrap();
        let mut other = GuestMemory::from_owned(0x8000, vec![0; 1024].into_boxed_slice()).unwrap();
        let foreign = other.allocate(256, 4, Purpose::RuntimeDeviceTree).unwrap();
        let stamp = ledger.snapshot().stamp();
        let before = state(&ledger);
        for (s, token, expected) in [
            (stamp, wrong, Error::WrongPurpose),
            (stamp, small, Error::DestinationCapacity),
            (stamp, unaligned, Error::UnalignedAddress),
            (stamp, foreign, Error::ForeignOwner),
            (other.snapshot().stamp(), small, Error::ForeignOwner),
        ] {
            assert_eq!(
                ledger
                    .bind_device_tree(s, token, tree(&source, 0x8000, 1024))
                    .unwrap_err(),
                expected
            );
            assert_eq!(state(&ledger), before);
        }
        ledger.release(small).unwrap();
        let current = ledger.snapshot().stamp();
        assert_eq!(
            ledger
                .bind_device_tree(stamp, small, tree(&source, 0x8000, 1024))
                .unwrap_err(),
            Error::StaleSnapshot
        );
        assert_eq!(
            ledger
                .bind_device_tree(current, small, tree(&source, 0x8000, 1024))
                .unwrap_err(),
            Error::ReleasedReservation
        );
    }
    assert_eq!(backing, [0x19; 1024]);
}

#[test]
fn stale_or_foreign_dt_commit_never_changes_ram() {
    let source = source_tree();
    for mutation in 0..6 {
        let mut backing = [0x62; 1024];
        let mut expected = backing;
        {
            let mut ledger = GuestMemory::from_borrowed(0x8000, &mut backing).unwrap();
            let target = ledger
                .allocate(512, 16, Purpose::RuntimeDeviceTree)
                .unwrap();
            let data = ledger.allocate(32, 16, Purpose::ProviderData).unwrap();
            let prepared = ledger
                .bind_device_tree(
                    ledger.snapshot().stamp(),
                    target,
                    tree(&source, 0x8000, 1024),
                )
                .unwrap();
            match mutation {
                0 => {
                    ledger.allocate(16, 16, Purpose::Stack).unwrap();
                }
                1 => {
                    ledger.release(data).unwrap();
                }
                2 => {
                    ledger.copy_into(data, 0, &[7]).unwrap();
                    expected[512] = 7;
                }
                3 => {
                    ledger.release(target).unwrap();
                    ledger
                        .allocate(512, 16, Purpose::RuntimeDeviceTree)
                        .unwrap();
                }
                4 => {
                    ledger.with_guest_memory(|_, _| ()).unwrap();
                }
                5 => {
                    let another = ledger
                        .bind_device_tree(
                            ledger.snapshot().stamp(),
                            target,
                            tree(&source, 0x8000, 1024),
                        )
                        .unwrap();
                    let bytes = another.bytes().to_vec();
                    ledger.commit_device_tree(another).unwrap();
                    expected[..bytes.len()].copy_from_slice(&bytes);
                }
                _ => unreachable!(),
            }
            let before = state(&ledger);
            assert_eq!(
                ledger.commit_device_tree(prepared),
                Err(Error::StaleSnapshot)
            );
            assert_eq!(state(&ledger), before);
        }
        assert_eq!(backing, expected, "mutation {mutation}");
    }
    let mut other_bytes = [0x21; 512];
    {
        let mut first = GuestMemory::from_owned(0x8000, vec![0; 512].into_boxed_slice()).unwrap();
        let token = first.allocate(512, 16, Purpose::RuntimeDeviceTree).unwrap();
        let prepared = first
            .bind_device_tree(first.snapshot().stamp(), token, tree(&source, 0x8000, 512))
            .unwrap();
        drop(first);
        let mut other = GuestMemory::from_borrowed(0x8000, &mut other_bytes).unwrap();
        other.allocate(512, 16, Purpose::RuntimeDeviceTree).unwrap();
        let before = state(&other);
        assert_eq!(other.commit_device_tree(prepared), Err(Error::ForeignOwner));
        assert_eq!(state(&other), before);
    }
    assert_eq!(other_bytes, [0x21; 512]);
}

#[test]
fn malformed_or_missing_provider_data_never_reaches_ledger_commit() {
    let source = source_tree();
    let mut bytes = [0x28; 512];
    {
        let mut ledger = GuestMemory::from_borrowed(0x4000, &mut bytes).unwrap();
        ledger.allocate(512, 4, Purpose::RuntimeDeviceTree).unwrap();
        let before = state(&ledger);
        assert!(matches!(
            runtime_dt::prepare(&source, &[], runtime_dt::MAX_OUTPUT_BYTES),
            Err(runtime_dt::Error::MissingProviders(_))
        ));
        assert!(matches!(
            runtime_dt::prepare(&source[..7], &[], runtime_dt::MAX_OUTPUT_BYTES),
            Err(runtime_dt::Error::Firmware(_))
        ));
        assert_eq!(state(&ledger), before);
    }
    assert_eq!(bytes, [0x28; 512]);
}

#[test]
fn scoped_service_can_read_dt_and_return_owned_state_but_errors_do_not_rollback() {
    struct Service<'a> {
        base: u64,
        ram: &'a mut [u8],
    }
    impl Service<'_> {
        fn read(&self, pa: u64, count: usize) -> &[u8] {
            let index = usize::try_from(pa - self.base).unwrap();
            &self.ram[index..index + count]
        }
    }
    let source = source_tree();
    let mut ledger =
        GuestMemory::from_owned(0x2000_0000, vec![0x51; 1024].into_boxed_slice()).unwrap();
    let dt = ledger
        .allocate(512, 16, Purpose::RuntimeDeviceTree)
        .unwrap();
    let marker = ledger.allocate(16, 16, Purpose::ProviderData).unwrap();
    let materialized = tree(
        &source,
        ledger.aperture().address(),
        ledger.aperture().bytes(),
    );
    let expected = materialized.bytes().to_vec();
    let prepared = ledger
        .bind_device_tree(ledger.snapshot().stamp(), dt, materialized)
        .unwrap();
    ledger.commit_device_tree(prepared).unwrap();
    let generation = ledger.snapshot().generation();
    let result: Result<(), u32> = ledger
        .with_guest_memory(|base, ram| {
            let service = Service { base, ram };
            flat_dt::validate(service.read(base, expected.len())).unwrap();
            assert_eq!(service.read(base, expected.len()), expected);
            service.ram[512..516].copy_from_slice(&0x1369_abefu32.to_le_bytes());
            Err(17)
        })
        .unwrap();
    assert_eq!(result, Err(17));
    assert_eq!(
        &ledger.read(marker).unwrap()[..4],
        &0x1369_abefu32.to_le_bytes()
    );
    assert_eq!(ledger.snapshot().generation(), generation + 1);
}
