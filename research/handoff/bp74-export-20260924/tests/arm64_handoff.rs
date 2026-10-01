use nextcore_core::{
    flat_dt::{self, FlatNode, FlatProperty},
    xnu_arm64_boot_args::Arm64BootVideo,
    xnu_arm64_handoff::{
        Arm64FramebufferGeometry as Geometry, Arm64HandoffError as E, Arm64HandoffPlan,
        Arm64PlacementInput,
    },
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

#[test]
fn framebuffer_reservation_wire_and_all_owned_intervals_agree() {
    let source = fixture();
    let original = source.clone();
    let tree = dt();
    // Padded rows and a partial final page distinguish pixels from reservation.
    let geometry = Geometry {
        width: 17,
        height: 257,
        row_bytes: 80,
    };
    let plan = Arm64HandoffPlan::new_with_framebuffer(&source, input(&tree), geometry).unwrap();
    let l = plan.layout();
    let fb = plan.framebuffer().unwrap();
    assert_eq!(fb.base_phys, PA + 9 * PAGE as u64);
    assert_eq!(fb.byte_len, 20560);
    assert_eq!(fb.reserved_bytes, 32768);
    assert_eq!(fb.geometry, geometry);
    assert_eq!(l.stack_top_phys, fb.base_phys);
    assert_eq!(l.occupied_end, PA + 11 * PAGE as u64);
    assert_eq!(l.allocation_bytes, 11 * PAGE);
    let regions = [
        (PA, PA + 3 * PAGE as u64),
        (l.boot_args_phys, l.boot_args_phys + PAGE as u64),
        (l.device_tree_phys, l.device_tree_phys + tree.len() as u64),
        (l.stack_top_phys - 4 * PAGE as u64, l.stack_top_phys),
        (fb.base_phys, fb.base_phys + fb.reserved_bytes),
    ];
    for (n, region) in regions.iter().enumerate() {
        assert!(region.0 < region.1 && region.1 <= l.occupied_end);
        for other in &regions[..n] {
            assert!(region.1 <= other.0 || other.1 <= region.0);
        }
    }
    // Independent serialized oracle: public LP64 offsets, not codec getters.
    let mut video = Vec::new();
    for value in [PA + 9 * PAGE as u64, 1, 80, 17, 257, 32] {
        video.extend_from_slice(&value.to_le_bytes());
    }
    let mut bytes = vec![0xa5; l.allocation_bytes];
    plan.stage_into(&mut bytes).unwrap();
    assert_eq!(&bytes[3 * PAGE + 40..3 * PAGE + 88], video);
    assert_eq!(
        &bytes[3 * PAGE + 32..3 * PAGE + 40],
        &(PA + 11 * PAGE as u64).to_le_bytes()
    );
    assert!(bytes[9 * PAGE..].iter().all(|&b| b == 0));
    assert_eq!(source, original);
    assert!(!plan.execution_ready());
}

#[test]
fn framebuffer_pixels_row_padding_and_page_padding_are_verified() {
    let source = fixture();
    let tree = dt();
    let plan = Arm64HandoffPlan::new_with_framebuffer(
        &source,
        input(&tree),
        Geometry {
            width: 3,
            height: 2,
            row_bytes: 16,
        },
    )
    .unwrap();
    let mut bytes = vec![0xa5; plan.layout().allocation_bytes];
    plan.stage_into(&mut bytes).unwrap();
    for offset in [
        9 * PAGE,
        9 * PAGE + 12,
        9 * PAGE + 31,
        9 * PAGE + 32,
        10 * PAGE - 1,
    ] {
        bytes[offset] = 0x7f;
        assert_eq!(plan.verify(&bytes), Err(E::ReadbackMismatch));
        bytes[offset] = 0;
    }
    plan.verify(&bytes).unwrap();
    assert_eq!(
        plan.verify(&bytes[..bytes.len() - 1]),
        Err(E::DestinationSize)
    );
}

#[test]
fn framebuffer_geometry_conflicts_and_width_overflow_fail_before_staging() {
    let source = fixture();
    let tree = dt();
    for geometry in [
        Geometry {
            width: 0,
            height: 1,
            row_bytes: 4,
        },
        Geometry {
            width: 1,
            height: 0,
            row_bytes: 4,
        },
        Geometry {
            width: 2,
            height: 1,
            row_bytes: 4,
        },
        Geometry {
            width: 1,
            height: 1,
            row_bytes: 5,
        },
        Geometry {
            width: u32::MAX,
            height: 1,
            row_bytes: u32::MAX - 3,
        },
    ] {
        assert!(matches!(
            Arm64HandoffPlan::new_with_framebuffer(&source, input(&tree), geometry),
            Err(E::InvalidFramebufferGeometry)
        ));
    }
    let geometry = Geometry {
        width: 1,
        height: 1,
        row_bytes: 4,
    };
    let mut i = input(&tree);
    i.video.base_address = PA;
    assert!(matches!(
        Arm64HandoffPlan::new_with_framebuffer(&source, i, geometry),
        Err(E::ConflictingVideo)
    ));
    i.video = Arm64BootVideo {
        depth: 32,
        ..Default::default()
    };
    assert!(matches!(
        Arm64HandoffPlan::new_with_framebuffer(&source, i, geometry),
        Err(E::ConflictingVideo)
    ));
}

#[test]
fn framebuffer_full_rounded_reservation_must_fit_with_free_ram_above() {
    let source = fixture();
    let tree = dt();
    let geometry = Geometry {
        width: 1,
        height: 1,
        row_bytes: 4,
    };
    let mut i = input(&tree);
    i.memory_size = 0x02000000 + 10 * PAGE as u64;
    // Legacy stack fits; a four-byte framebuffer still reserves a complete page.
    assert!(Arm64HandoffPlan::new(&source, i).is_ok());
    assert!(matches!(
        Arm64HandoffPlan::new_with_framebuffer(&source, i, geometry),
        Err(E::BootArgs(_))
    ));
    i.memory_size += PAGE as u64;
    assert!(Arm64HandoffPlan::new_with_framebuffer(&source, i, geometry).is_ok());
    let huge = Geometry {
        width: 1,
        height: u32::MAX,
        row_bytes: u32::MAX - 3,
    };
    assert!(matches!(
        Arm64HandoffPlan::new_with_framebuffer(&source, input(&tree), huge),
        Err(E::AddressOverflow)
    ));
}

#[test]
fn framebuffer_address_addition_wrap_is_rejected() {
    let source = fixture();
    let tree = dt();
    let mut i = input(&tree);
    // Keep the verified VA/PA block correspondence while placing the stack near
    // the end of u64. Appending a large valid framebuffer must not wrap to zero.
    i.physical_base = 0xffff_ffff_fc00_0000;
    i.kernel_phys = 0xffff_ffff_fe00_0000;
    let geometry = Geometry {
        width: 8192,
        height: 8192,
        row_bytes: 32768,
    };
    assert!(matches!(
        Arm64HandoffPlan::new_with_framebuffer(&source, i, geometry),
        Err(E::AddressOverflow)
    ));
}

#[test]
fn legacy_layout_and_unmanaged_video_remain_unchanged() {
    let source = fixture();
    let tree = dt();
    let mut i = input(&tree);
    i.video = Arm64BootVideo {
        base_address: 0x12345678,
        display: 9,
        row_bytes: 100,
        width: 20,
        height: 30,
        depth: 16,
    };
    let plan = Arm64HandoffPlan::new(&source, i).unwrap();
    assert!(plan.framebuffer().is_none());
    assert_eq!(plan.layout().stack_top_phys, PA + 9 * PAGE as u64);
    assert_eq!(plan.layout().occupied_end, plan.layout().stack_top_phys);
    assert_eq!(plan.layout().allocation_bytes, 9 * PAGE);
    let mut expected = Vec::new();
    for value in [0x12345678u64, 9, 100, 20, 30, 16] {
        expected.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(&plan.boot_args().as_bytes()[40..88], expected);
}
