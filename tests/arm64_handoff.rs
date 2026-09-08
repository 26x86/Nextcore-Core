use nextcore_core::{
    flat_dt::{self, FlatNode, FlatProperty},
    xnu_arm64_boot_args::Arm64BootVideo,
    xnu_arm64_handoff::{Arm64HandoffError as E, Arm64HandoffPlan, Arm64PlacementInput},
};
const PAGE: usize = 16384;
const VA: u64 = 0xffff_fe00_0200_0000;
const PA: u64 = 0x4200_0000;
fn put32(b: &mut [u8], at: usize, value: u32) {
    b[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, value: u64) {
    b[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn segment(offset: usize, file: usize, memory: usize, protection: u32) -> Vec<u8> {
    let mut b = vec![0; 72];
    put32(&mut b, 0, 0x19);
    put32(&mut b, 4, 72);
    b[8..14].copy_from_slice(b"__TEXT");
    put64(&mut b, 24, VA + offset as u64);
    put64(&mut b, 32, memory as u64);
    put64(&mut b, 40, offset as u64);
    put64(&mut b, 48, file as u64);
    put32(&mut b, 56, protection);
    put32(&mut b, 60, protection);
    b
}
fn header(bytes: &mut [u8], offset: usize, kind: u32, commands: Vec<Vec<u8>>) {
    put32(bytes, offset, 0xfeedfacf);
    put32(bytes, offset + 4, 0x0100000c);
    put32(bytes, offset + 12, kind);
    put32(bytes, offset + 16, commands.len() as u32);
    put32(
        bytes,
        offset + 20,
        commands.iter().map(Vec::len).sum::<usize>() as u32,
    );
    let mut cursor = offset + 32;
    for c in commands {
        bytes[cursor..cursor + c.len()].copy_from_slice(&c);
        cursor += c.len();
    }
}
fn fixture() -> Vec<u8> {
    let mut b = vec![0; 2 * PAGE];
    let mut thread = vec![0; 288];
    put32(&mut thread, 0, 5);
    put32(&mut thread, 4, 288);
    put32(&mut thread, 8, 6);
    put32(&mut thread, 12, 68);
    put64(&mut thread, 272, VA + PAGE as u64 + 1024);
    let mut member = vec![0; 48];
    put32(&mut member, 0, 0x80000035);
    put32(&mut member, 4, 48);
    put64(&mut member, 8, VA + PAGE as u64);
    put64(&mut member, 16, PAGE as u64);
    put32(&mut member, 24, 32);
    member[32..39].copy_from_slice(b"fixture");
    header(
        &mut b,
        0,
        12,
        vec![
            segment(0, PAGE, PAGE, 1),
            segment(PAGE, PAGE, 2 * PAGE, 5),
            thread,
            member,
        ],
    );
    header(&mut b, PAGE, 2, vec![segment(PAGE, PAGE, 2 * PAGE, 5)]);
    b[PAGE + 1024..PAGE + 1028].copy_from_slice(&0xd4400000u32.to_le_bytes());
    b
}
fn dt() -> Vec<u8> {
    flat_dt::encode(&FlatNode {
        name: String::new(),
        properties: vec![],
        children: vec![FlatNode {
            name: "chosen".into(),
            properties: vec![
                FlatProperty {
                    name: "dram-base".into(),
                    value: 0x40000000u64.to_le_bytes().to_vec(),
                },
                FlatProperty {
                    name: "dram-size".into(),
                    value: 0x20000000u64.to_le_bytes().to_vec(),
                },
            ],
            children: vec![],
        }],
    })
    .unwrap()
}
fn input(dt: &[u8]) -> Arm64PlacementInput<'_> {
    Arm64PlacementInput {
        physical_base: 0x40000000,
        virtual_base: 0xfffffe0000000000,
        memory_size: 0x20000000,
        actual_memory_size: 0x20000000,
        kernel_phys: PA,
        device_tree: dt,
        command_line: "-v",
        machine_type: 0,
        boot_flags: 0,
        video: Arm64BootVideo::default(),
    }
}
#[test]
fn physical_layout_stage_and_wire_coordinates_agree() {
    let source = fixture();
    let before = source.clone();
    let dt = dt();
    let plan = Arm64HandoffPlan::new(&source, input(&dt)).unwrap();
    let l = plan.layout();
    assert_eq!(l.kernel_bytes, 3 * PAGE);
    assert_eq!(l.entry_phys, PA + PAGE as u64 + 1024);
    assert_eq!(l.collection_header_phys, PA);
    assert_eq!(l.boot_args_phys, PA + (3 * PAGE) as u64);
    assert_eq!(l.device_tree_phys, PA + (4 * PAGE) as u64);
    assert_eq!(l.occupied_end, PA + (9 * PAGE) as u64);
    assert_eq!(plan.boot_args().physical_address(), l.boot_args_phys);
    assert_eq!(
        plan.boot_args().device_tree_virtual_address(),
        VA + (4 * PAGE) as u64
    );
    let mut guest = vec![0xa5; l.allocation_bytes];
    let receipt = plan.stage_into(&mut guest).unwrap();
    assert_eq!(receipt.zero_tail_bytes, PAGE);
    assert_eq!(
        &guest[3 * PAGE..3 * PAGE + 1152],
        plan.boot_args().as_bytes()
    );
    assert_eq!(&guest[4 * PAGE..4 * PAGE + dt.len()], dt);
    assert!(guest[5 * PAGE..].iter().all(|b| *b == 0));
    assert_eq!(source, before);
    assert!(!plan.execution_ready());
}
#[test]
fn modified_code_arguments_tree_padding_and_stack_are_detected() {
    let source = fixture();
    let dt = dt();
    let plan = Arm64HandoffPlan::new(&source, input(&dt)).unwrap();
    let mut guest = vec![0; plan.layout().allocation_bytes];
    plan.stage_into(&mut guest).unwrap();
    for offset in [
        PAGE + 1024,
        3 * PAGE + 108,
        3 * PAGE + 1152,
        4 * PAGE + 8,
        5 * PAGE,
        guest.len() - 1,
    ] {
        guest[offset] ^= 1;
        assert!(plan.verify(&guest).is_err(), "offset={offset}");
        guest[offset] ^= 1;
        assert!(plan.verify(&guest).is_ok());
    }
    let short = guest.len() - 1;
    assert_eq!(plan.verify(&guest[..short]), Err(E::DestinationSize));
}
#[test]
fn invalid_linear_mapping_alignment_and_ram_coverage_are_rejected() {
    let source = fixture();
    let dt = dt();
    let mut i = input(&dt);
    i.kernel_phys += 1;
    assert!(matches!(
        Arm64HandoffPlan::new(&source, i),
        Err(E::InvalidPlacement)
    ));
    let mut i = input(&dt);
    i.virtual_base += PAGE as u64;
    assert!(matches!(
        Arm64HandoffPlan::new(&source, i),
        Err(E::BootstrapCorrespondence)
    ));
    let mut i = input(&dt);
    i.memory_size = 0x02008000;
    assert!(matches!(
        Arm64HandoffPlan::new(&source, i),
        Err(E::BootArgs(_))
    ));
    let mut i = input(&dt);
    i.virtual_base = u64::MAX - 0x1000;
    assert!(matches!(
        Arm64HandoffPlan::new(&source, i),
        Err(E::AddressOverflow)
    ));
    let large = vec![0; 1024 * 1024 + 4];
    assert!(matches!(
        Arm64HandoffPlan::new(&source, input(&large)),
        Err(E::DeviceTreeTooLarge)
    ));
}
