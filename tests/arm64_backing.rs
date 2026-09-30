use nextcore_core::xnu_arm64_backing::{Arm64BackingError as E, Arm64HandoffBacking};
use nextcore_core::xnu_arm64_handoff::Arm64HandoffLayout;

const PAGE: usize = 16 * 1024;
const RAM_BASE: u64 = 0x4200_0000;
const TABLE_BASE: u64 = 0x1000_0000;

fn layout() -> Arm64HandoffLayout {
    Arm64HandoffLayout {
        kernel_phys: RAM_BASE,
        kernel_bytes: 3 * PAGE,
        collection_header_phys: RAM_BASE,
        entry_phys: RAM_BASE + 1024,
        boot_args_phys: RAM_BASE + (3 * PAGE) as u64,
        device_tree_phys: RAM_BASE + (4 * PAGE) as u64,
        stack_top_phys: RAM_BASE + (9 * PAGE) as u64,
        occupied_end: RAM_BASE + (9 * PAGE) as u64,
        allocation_bytes: 9 * PAGE,
    }
}

#[test]
fn borrowed_backing_rejects_missing_tail_and_table_alias_before_write() {
    let plan = layout();
    let mut short = vec![0xa5; 8 * PAGE];
    let tables = vec![0; PAGE];
    assert_eq!(
        Arm64HandoffBacking::new(&plan, &mut short, RAM_BASE, &tables, TABLE_BASE).err(),
        Some(E::InsufficientRam)
    );
    assert!(short.iter().all(|&byte| byte == 0xa5));

    let mut forged_unaligned_kernel = plan;
    forged_unaligned_kernel.kernel_bytes -= 1;
    forged_unaligned_kernel.boot_args_phys -= 1;
    forged_unaligned_kernel.device_tree_phys -= 1;
    assert_eq!(
        Arm64HandoffBacking::new(
            &forged_unaligned_kernel,
            &mut short,
            RAM_BASE,
            &tables,
            TABLE_BASE,
        )
        .err(),
        Some(E::InvalidLayout)
    );
    assert!(short.iter().all(|&byte| byte == 0xa5));

    let mut forged_short_stack = plan;
    forged_short_stack.stack_top_phys = RAM_BASE + (8 * PAGE) as u64;
    forged_short_stack.occupied_end = forged_short_stack.stack_top_phys;
    forged_short_stack.allocation_bytes = 8 * PAGE;
    assert_eq!(
        Arm64HandoffBacking::new(
            &forged_short_stack,
            &mut short,
            RAM_BASE,
            &tables,
            TABLE_BASE,
        )
        .err(),
        Some(E::InvalidLayout)
    );
    assert!(short.iter().all(|&byte| byte == 0xa5));

    let mut ram = vec![0xa5; 9 * PAGE];
    assert_eq!(
        Arm64HandoffBacking::new(&plan, &mut ram, RAM_BASE, &tables, RAM_BASE).err(),
        Some(E::OverlappingGuestRanges)
    );
    assert_eq!(
        Arm64HandoffBacking::new(&plan, &mut ram, RAM_BASE, &tables, TABLE_BASE + 1).err(),
        Some(E::InvalidRange)
    );
    let mut backing =
        Arm64HandoffBacking::new(&plan, &mut ram, RAM_BASE, &tables, TABLE_BASE).unwrap();
    assert_eq!(backing.handoff_mut().len(), 9 * PAGE);
    let (service_ram, base, service_tables, table_base) = backing.service_parts();
    assert_eq!(
        (service_ram.len(), base, service_tables.len(), table_base),
        (9 * PAGE, RAM_BASE, PAGE, TABLE_BASE)
    );
    assert!(service_ram.iter().all(|&byte| byte == 0xa5));
}
