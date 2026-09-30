//! Authored public-format Mach-O fixtures; no selected companion bytes appear here.

use nextcore_core::arm64_startup_image::{stage_startup_images, StartupImageError as E};
use nextcore_core::arm64_stage1_tables::{Arm64Stage1Tables, Stage1Alias};

const LINKED_BASE: u64 = 0xffff_fe00_0000_0000;
const PHYSICAL_BASE: u64 = 0x4000_0000;

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn segment(vmaddr: u64, vmsize: u64, fileoff: u64, filesize: u64) -> Vec<u8> {
    let mut command = vec![0; 72];
    put32(&mut command, 0, 0x19);
    put32(&mut command, 4, 72);
    put64(&mut command, 24, vmaddr);
    put64(&mut command, 32, vmsize);
    put64(&mut command, 40, fileoff);
    put64(&mut command, 48, filesize);
    put32(&mut command, 56, 5);
    put32(&mut command, 60, 5);
    command
}

fn fixture_at(linked_base: u64) -> Vec<u8> {
    let mut commands = vec![
        segment(linked_base, 0x4000, 0, 0x800),
        segment(linked_base + 0x8000, 0x5000, 0x800, 0x200),
    ];
    let mut symtab = vec![0; 24];
    put32(&mut symtab, 0, 0x2);
    put32(&mut symtab, 4, 24);
    put32(&mut symtab, 8, 0x900);
    put32(&mut symtab, 12, 1);
    put32(&mut symtab, 16, 0x920);
    put32(&mut symtab, 20, 8);
    commands.push(symtab);
    let mut dysymtab = vec![0; 80];
    put32(&mut dysymtab, 0, 0xb);
    put32(&mut dysymtab, 4, 80);
    put32(&mut dysymtab, 8, 0);
    put32(&mut dysymtab, 12, 1);
    commands.push(dysymtab);
    let mut uuid = vec![0; 24];
    put32(&mut uuid, 0, 0x1b);
    put32(&mut uuid, 4, 24);
    commands.push(uuid);
    let mut version = vec![0; 16];
    put32(&mut version, 0, 0x2a);
    put32(&mut version, 4, 16);
    commands.push(version);
    let mut thread = vec![0; 288];
    put32(&mut thread, 0, 0x5);
    put32(&mut thread, 4, 288);
    put32(&mut thread, 8, 6);
    put32(&mut thread, 12, 68);
    commands.push(thread);

    let command_bytes: usize = commands.iter().map(Vec::len).sum();
    let mut bytes = vec![0; 0xa00];
    put32(&mut bytes, 0, 0xfeed_facf);
    put32(&mut bytes, 4, 0x0100_000c);
    put32(&mut bytes, 8, 0x8000_0002);
    put32(&mut bytes, 12, 2);
    put32(&mut bytes, 16, commands.len() as u32);
    put32(&mut bytes, 20, command_bytes as u32);
    put32(&mut bytes, 24, 0x20_0001);
    let mut offset = 32;
    for command in commands {
        bytes[offset..offset + command.len()].copy_from_slice(&command);
        offset += command.len();
    }
    bytes[0x800..0xa00].fill(0x5a);
    bytes
}

fn fixture() -> Vec<u8> {
    fixture_at(LINKED_BASE)
}

fn linked_pa(tables: &Arm64Stage1Tables, va: u64) -> Option<u64> {
    let mut table = tables.ttbr1();
    for shift in [36, 25, 14] {
        let offset = usize::try_from(table.checked_sub(tables.physical_base())?).ok()?
            + (((va >> shift) & 0x7ff) as usize) * 8;
        let descriptor = u64::from_le_bytes(tables.bytes().get(offset..offset + 8)?.try_into().ok()?);
        if descriptor & 3 != 3 {
            return None;
        }
        let address = descriptor & 0x0000_ffff_ffff_c000;
        if shift == 14 {
            return Some(address + (va & 0x3fff));
        }
        table = address;
    }
    None
}

#[test]
fn stages_two_images_sequentially_and_zeroes_holes_and_tails() {
    let sptm = fixture();
    let txm_linked_base = LINKED_BASE + 0x2000_0000;
    let mut txm = fixture_at(txm_linked_base);
    txm[0x800..0xa00].fill(0xa5);
    let mut memory = vec![0xcc; 0x30000];
    let placement = stage_startup_images(
        &sptm,
        &txm,
        PHYSICAL_BASE,
        PHYSICAL_BASE + 0x123,
        &mut memory,
    )
    .unwrap();
    assert_eq!(placement.sptm.physical_base, PHYSICAL_BASE + 0x4000);
    assert_eq!(placement.sptm.virtual_base, LINKED_BASE);
    assert_eq!(placement.sptm.bytes, 0x10000);
    assert_eq!(placement.sptm.copied_bytes, 0xa00);
    assert_eq!(placement.txm.physical_base, PHYSICAL_BASE + 0x14000);
    assert_eq!(placement.txm.virtual_base, txm_linked_base);
    assert_eq!(placement.txm.bytes, 0x10000);
    assert_eq!(placement.occupied_end, PHYSICAL_BASE + 0x24000);
    assert!(memory[..0x4000].iter().all(|&b| b == 0xcc));
    assert_eq!(&memory[0x4000..0x4800], &sptm[..0x800]);
    assert!(memory[0x4800..0xc000].iter().all(|&b| b == 0));
    assert_eq!(&memory[0xc000..0xc200], &sptm[0x800..0xa00]);
    assert!(memory[0xc200..0x14000].iter().all(|&b| b == 0));
    assert_eq!(&memory[0x1c000..0x1c200], &txm[0x800..0xa00]);
    assert!(memory[0x24000..].iter().all(|&b| b == 0xcc));
    let aliases = [placement.sptm, placement.txm].map(|image| Stage1Alias {
        virtual_base: image.virtual_base,
        physical_base: image.physical_base,
        bytes: image.bytes as u64,
    });
    let tables = Arm64Stage1Tables::new_with_aliases(
        PHYSICAL_BASE,
        0xffff_fd00_0000_0000,
        memory.len() as u64,
        &aliases,
    )
    .unwrap();
    for alias in aliases {
        assert_eq!(linked_pa(&tables, alias.virtual_base), Some(alias.physical_base));
        assert_eq!(
            linked_pa(&tables, alias.virtual_base + alias.bytes - 1),
            Some(alias.physical_base + alias.bytes - 1)
        );
    }
}

#[test]
fn rejects_virtual_and_file_overlap_without_touching_memory() {
    let original = fixture();
    let mut virtual_overlap = original.clone();
    put64(&mut virtual_overlap, 32 + 72 + 24, LINKED_BASE + 0x3000);
    let mut memory = vec![0xcc; 0x30000];
    assert_eq!(
        stage_startup_images(
            &original,
            &virtual_overlap,
            PHYSICAL_BASE,
            PHYSICAL_BASE,
            &mut memory
        ),
        Err(E::OverlappingRange)
    );
    assert!(memory.iter().all(|&b| b == 0xcc));

    let mut file_overlap = original.clone();
    put64(&mut file_overlap, 32 + 72 + 40, 0x700);
    assert_eq!(
        stage_startup_images(
            &file_overlap,
            &original,
            PHYSICAL_BASE,
            PHYSICAL_BASE,
            &mut memory
        ),
        Err(E::OverlappingRange)
    );
    assert!(memory.iter().all(|&b| b == 0xcc));
}

#[test]
fn second_span_arena_failure_is_transactional() {
    let image = fixture();
    let mut memory = vec![0xcc; 0x1e000];
    assert_eq!(
        stage_startup_images(
            &image,
            &image,
            PHYSICAL_BASE,
            PHYSICAL_BASE + 0x123,
            &mut memory
        ),
        Err(E::ArenaBounds)
    );
    assert!(memory.iter().all(|&b| b == 0xcc));
}

#[test]
fn changed_header_and_file_ranges_are_rejected_before_staging() {
    let original = fixture();
    let mut memory = vec![0xcc; 0x30000];
    let mut changed = original.clone();
    put32(&mut changed, 8, 0);
    assert_eq!(
        stage_startup_images(
            &changed,
            &original,
            PHYSICAL_BASE,
            PHYSICAL_BASE,
            &mut memory
        ),
        Err(E::UnsupportedCpu)
    );
    let mut changed = original.clone();
    put64(&mut changed, 32 + 72 + 40, 0x9ff);
    assert_eq!(
        stage_startup_images(
            &original,
            &changed,
            PHYSICAL_BASE,
            PHYSICAL_BASE,
            &mut memory
        ),
        Err(E::Truncated)
    );
    assert!(memory.iter().all(|&b| b == 0xcc));
}

#[test]
fn section_file_range_must_remain_inside_its_segment() {
    let mut image = fixture();
    let command_end = 32 + 72 * 2 + 24 + 80 + 24 + 16 + 288;
    image.copy_within(32 + 72..command_end, 32 + 152);
    image[32 + 72..32 + 152].fill(0);
    put32(&mut image, 32 + 4, 152);
    put32(&mut image, 32 + 64, 1);
    put32(&mut image, 20, (command_end + 80 - 32) as u32);
    put64(&mut image, 32 + 72 + 32, LINKED_BASE + 0x200);
    put64(&mut image, 32 + 72 + 40, 0x100);
    put32(&mut image, 32 + 72 + 48, 0x200);
    let mut memory = vec![0xcc; 0x30000];
    assert!(stage_startup_images(
        &image,
        &fixture(),
        PHYSICAL_BASE,
        PHYSICAL_BASE,
        &mut memory
    )
    .is_ok());

    put32(&mut image, 32 + 72 + 48, 0x900);
    memory.fill(0xcc);
    assert_eq!(
        stage_startup_images(
            &image,
            &fixture(),
            PHYSICAL_BASE,
            PHYSICAL_BASE,
            &mut memory
        ),
        Err(E::InvalidSegment)
    );
    assert!(memory.iter().all(|&b| b == 0xcc));
}
