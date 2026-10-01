use nextcore_core::arm64_stage1_tables::{
    Arm64Stage1Tables, Error, MAX_MEMORY_SIZE, MAX_TABLE_BYTES,
};

// Independent descriptor reader: fail on non-table ancestors or non-page leaf,
// check every attribute bit, and resolve only inside the owned table allocation.
fn walk(t: &Arm64Stage1Tables, va: u64) -> Option<u64> {
    let mut base = if va >> 47 == 0 {
        t.ttbr0()
    } else if va >> 47 == 0x1ffff {
        t.ttbr1()
    } else {
        return None;
    };
    for (level, shift) in [(1, 36), (2, 25), (3, 14)] {
        let at = usize::try_from(base.checked_sub(t.physical_base())?).ok()?
            + usize::try_from((va >> shift) % 2048).ok()? * 8;
        let d = u64::from_le_bytes(t.bytes().get(at..at + 8)?.try_into().ok()?);
        if d & 3 != 3 {
            return None;
        }
        let address = d & 0x0000_ffff_ffff_c000;
        if level == 3 {
            assert_eq!(d ^ address, 0x403, "AF/Attr0/nonshareable/EL1 RWX only");
            return Some(address + va % 16384);
        }
        assert_eq!(d ^ address, 3);
        base = address;
    }
    None
}

#[test]
fn both_aliases_cover_every_page_and_no_other_pages() {
    // Both aliases straddle L1 and L2 boundaries, at different offsets.
    let pa = (1 << 36) - 16384;
    let va = 0xffff_9000_0000_0000 - 32768;
    let size = 64 * 1024 * 1024;
    let t = Arm64Stage1Tables::new(pa, va, size).unwrap();
    assert_eq!(t.physical_base(), pa + size);
    assert_eq!(t.ttbr1(), t.ttbr0() + 16384);
    assert_eq!(t.tcr(), 0x540118011);
    assert!(t.bytes().len() <= MAX_TABLE_BYTES);
    for offset in (0..size).step_by(16384) {
        for byte in [0, 4, 16383] {
            assert_eq!(walk(&t, pa + offset + byte), Some(pa + offset + byte));
            assert_eq!(walk(&t, va + offset + byte), Some(pa + offset + byte));
        }
    }
    for address in [
        pa - 1,
        pa + size,
        va - 1,
        va + size,
        t.ttbr0(),
        t.ttbr1(),
        1 << 47,
    ] {
        assert_eq!(walk(&t, address), None);
    }
    assert_eq!(t.virtual_address(pa + size - 1), Ok(va + size - 1));
    assert_eq!(t.virtual_address(pa - 1), Err(Error::UnmappedAddress));
    assert_eq!(t.virtual_address(pa + size), Err(Error::UnmappedAddress));
}

#[test]
fn maximum_ram_and_canonical_edges_remain_bounded() {
    let size = MAX_MEMORY_SIZE;
    let pa = (1 << 47) - size;
    let va = 0xffff_8000_0000_0000;
    let t = Arm64Stage1Tables::new(pa, va, size).unwrap();
    assert!(t.bytes().len() <= MAX_TABLE_BYTES);
    assert_eq!(walk(&t, pa + size - 1), Some(pa + size - 1));
    assert_eq!(walk(&t, va + size - 1), Some(pa + size - 1));
    assert_eq!(walk(&t, pa + size), None);
}

#[test]
fn rejects_alignment_empty_oversize_noncanonical_and_overflow() {
    let high = 0xffff_8000_0000_0000;
    for (pa, va, n) in [(1, high, 16384), (0, high + 1, 16384), (0, high, 16385)] {
        assert_eq!(
            Arm64Stage1Tables::new(pa, va, n).unwrap_err(),
            Error::Alignment
        );
    }
    for (pa, va, n) in [
        (0, high, 0),
        (0, high, MAX_MEMORY_SIZE + 16384),
        (1 << 47, high, 16384),
        (0, 0, 16384),
        (0, high - 16384, 16384),
        ((1 << 47) - 16384, high, 32768),
        (1 << 48, high, 16384),
    ] {
        assert_eq!(
            Arm64Stage1Tables::new(pa, va, n).unwrap_err(),
            Error::InvalidRange
        );
    }
    for (pa, va) in [(u64::MAX - 16383, high), (0, u64::MAX - 16383)] {
        assert_eq!(
            Arm64Stage1Tables::new(pa, va, 16384).unwrap_err(),
            Error::AddressOverflow
        );
    }
}
