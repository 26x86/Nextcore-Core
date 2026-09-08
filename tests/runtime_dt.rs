use nextcore_core::runtime_dt::*;
use nextcore_core::{firmware_dt::FirmwareDeviceTree, flat_dt};

fn property(name: &str, value: &[u8], template: bool) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    bytes.extend_from_slice(
        &((value.len() as u32) | if template { 1 << 31 } else { 0 }).to_le_bytes(),
    );
    bytes.extend_from_slice(value);
    bytes.resize((bytes.len() + 3) & !3, 0);
    bytes
}

fn node(properties: &[Vec<u8>], children: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(properties.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(children.len() as u32).to_le_bytes());
    for property in properties {
        bytes.extend_from_slice(property);
    }
    for child in children {
        bytes.extend_from_slice(child);
    }
    bytes
}

fn fixture() -> Vec<u8> {
    node(
        &[
            property("name", b"\0", false),
            property("literal", &[0xfe, 0x12, 0x80], false),
        ],
        &[node(
            &[
                property("name", b"chosen\0", false),
                property("dram-base", b"authored::ram.base()\0", true),
                property("dram-size", b"DO_NOT_EVALUATE/1+2\0", true),
            ],
            &[],
        )],
    )
}

fn binding(id: PropertyId, value: &[u8]) -> ProvidedValue<'_> {
    ProvidedValue {
        property: id,
        provider: "authored/observed-backing-v1",
        value,
    }
}

fn atomic_error(
    source: &[u8],
    provided: &[ProvidedValue<'_>],
    limit: usize,
    base: u64,
    reservation: DeclaredReservation,
) -> Error {
    let source_before = source.to_vec();
    let mut destination = [0xa5; 2048];
    let destination_before = destination;
    let result = materialize_into(source, provided, limit, &mut destination, base, reservation);
    assert_eq!(source, source_before);
    assert_eq!(destination, destination_before);
    result.unwrap_err()
}

fn room() -> DeclaredReservation {
    DeclaredReservation {
        address: 0x4080,
        capacity: 1024,
    }
}

#[test]
fn nested_explicit_values_preserve_literals_and_ignore_expression_bodies() {
    let source = fixture();
    let ids = template_ids(&source).unwrap();
    let values = [
        binding(ids[0], &[0, 1, 2, 3, 4, 5, 6, 7]),
        binding(ids[1], b"longer-authored-value\0"),
    ];
    let result = prepare(&source, &values, MAX_OUTPUT_BYTES).unwrap();
    assert_eq!(result.source(), source);
    assert_eq!(result.source_sha256(), source_identity(&source));
    assert_eq!(result.replacement_count(), 2);
    let literal = property("literal", &[0xfe, 0x12, 0x80], false);
    assert!(result
        .bytes()
        .windows(literal.len())
        .any(|bytes| bytes == literal));
    assert!(!result
        .bytes()
        .windows(b"DO_NOT_EVALUATE/".len())
        .any(|bytes| bytes == b"DO_NOT_EVALUATE/"));
    let expected = node(
        &[property("name", b"\0", false), literal],
        &[node(
            &[
                property("name", b"chosen\0", false),
                property("dram-base", values[0].value, false),
                property("dram-size", values[1].value, false),
            ],
            &[],
        )],
    );
    assert_eq!(result.bytes(), expected);
    assert_eq!(
        FirmwareDeviceTree::parse(result.bytes())
            .unwrap()
            .statistics()
            .templates,
        0
    );
    flat_dt::validate(result.bytes()).unwrap();
    assert_eq!(
        result.bindings()[1].value_sha256,
        source_identity(values[1].value)
    );
    assert_eq!(result.bindings()[1].provider, values[1].provider);
    assert_eq!(result.bindings()[1].value_bytes, values[1].value.len());
}

#[test]
fn all_missing_providers_are_returned_without_writes() {
    let source = fixture();
    let ids = template_ids(&source).unwrap();
    assert_eq!(
        atomic_error(&source, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::MissingProviders(ids.clone())
    );
    assert_eq!(
        atomic_error(
            &source,
            &[binding(ids[0], b"actual")],
            MAX_OUTPUT_BYTES,
            0x4000,
            room()
        ),
        Error::MissingProviders(vec![ids[1]])
    );
}

#[test]
fn source_identity_unknown_targets_duplicate_and_missing_labels_fail_atomically() {
    let source = fixture();
    let ids = template_ids(&source).unwrap();
    let mut bad_hash = ids[0];
    bad_hash.source_sha256[0] ^= 1;
    assert_eq!(
        atomic_error(
            &source,
            &[binding(bad_hash, b"v")],
            MAX_OUTPUT_BYTES,
            0x4000,
            room()
        ),
        Error::SourceIdentity
    );
    let mut literal_target = ids[0];
    literal_target.property_offset = 8;
    assert_eq!(
        atomic_error(
            &source,
            &[binding(literal_target, b"v")],
            MAX_OUTPUT_BYTES,
            0x4000,
            room()
        ),
        Error::UnknownBinding
    );
    let duplicates = [binding(ids[0], b"one"), binding(ids[0], b"two")];
    assert_eq!(
        atomic_error(&source, &duplicates, MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::DuplicateBinding
    );
    for label in ["", "has a space", "line\nfeed"] {
        let invalid = [ProvidedValue {
            property: ids[0],
            provider: label,
            value: b"v",
        }];
        assert_eq!(
            atomic_error(&source, &invalid, MAX_OUTPUT_BYTES, 0x4000, room()),
            Error::InvalidProvider
        );
    }
}

#[test]
fn literal_tree_is_unchanged_and_zero_length_is_only_an_explicit_value() {
    let source = node(
        &[
            property("name", b"\0", false),
            property("empty", &[], false),
        ],
        &[],
    );
    assert_eq!(
        prepare(&source, &[], MAX_OUTPUT_BYTES).unwrap().bytes(),
        source
    );
    let source = node(
        &[
            property("name", b"\0", false),
            property("empty", b"opaque\0", true),
        ],
        &[],
    );
    let id = template_ids(&source).unwrap()[0];
    assert!(matches!(
        prepare(&source, &[], MAX_OUTPUT_BYTES),
        Err(Error::MissingProviders(_))
    ));
    let value = [binding(id, &[])];
    assert_eq!(
        prepare(&source, &value, MAX_OUTPUT_BYTES)
            .unwrap()
            .bindings()[0]
            .value_bytes,
        0
    );
}

#[test]
fn source_truncation_counts_structural_templates_and_bad_padding_never_commit() {
    let valid = node(&[property("name", b"\0", false)], &[]);
    for length in 0..valid.len() {
        assert!(matches!(
            atomic_error(&valid[..length], &[], MAX_OUTPUT_BYTES, 0x4000, room()),
            Error::Firmware(_)
        ));
    }
    let mut huge_count = valid.clone();
    huge_count[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        atomic_error(&huge_count, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Firmware(_)
    ));
    let structural = node(&[property("name", b"opaque\0", true)], &[]);
    assert_eq!(
        atomic_error(&structural, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::StructuralTemplate
    );
    let mut padding = valid.clone();
    *padding.last_mut().unwrap() = 0x77;
    assert!(matches!(
        atomic_error(&padding, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Runtime(flat_dt::FlatDtError::InvalidPadding)
    ));
    let duplicate = node(
        &[
            property("name", b"\0", false),
            property("name", b"\0", false),
        ],
        &[],
    );
    assert!(matches!(
        atomic_error(&duplicate, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Runtime(flat_dt::FlatDtError::DuplicateProperty)
    ));
}

#[test]
fn arithmetic_output_limit_and_reservation_failures_are_atomic() {
    let source = node(&[property("name", b"\0", false)], &[]);
    assert_eq!(
        property_wire_bytes(u64::MAX),
        Err(Error::ArithmeticOverflow)
    );
    assert_eq!(
        property_wire_bytes(u64::from(1u32 << 31)),
        Err(Error::Limits)
    );
    for maximum in [0, source.len() - 1, MAX_OUTPUT_BYTES + 1] {
        assert_eq!(
            atomic_error(&source, &[], maximum, 0x4000, room()),
            Error::Limits
        );
    }
    for reservation in [
        DeclaredReservation {
            address: 0x4081,
            capacity: 512,
        },
        DeclaredReservation {
            address: 0x3ffc,
            capacity: 512,
        },
        DeclaredReservation {
            address: 0x47fc,
            capacity: 8,
        },
        DeclaredReservation {
            address: 0x4000,
            capacity: 0,
        },
    ] {
        assert_eq!(
            atomic_error(&source, &[], MAX_OUTPUT_BYTES, 0x4000, reservation),
            Error::InvalidReservation
        );
    }
    assert_eq!(
        atomic_error(
            &source,
            &[],
            MAX_OUTPUT_BYTES,
            0x4000,
            DeclaredReservation {
                address: 0x4080,
                capacity: 4
            }
        ),
        Error::DestinationCapacity
    );
    assert_eq!(
        atomic_error(
            &source,
            &[],
            MAX_OUTPUT_BYTES,
            0x4000,
            DeclaredReservation {
                address: u64::MAX - 3,
                capacity: 8
            }
        ),
        Error::ArithmeticOverflow
    );
    assert_eq!(
        atomic_error(&source, &[], MAX_OUTPUT_BYTES, u64::MAX - 1023, room()),
        Error::ArithmeticOverflow
    );
    assert_eq!(RamExtent::from_backing(0, &[]), Err(Error::InvalidRam));
}

#[test]
fn noncanonical_names_and_duplicate_children_are_rejected_before_commit() {
    let valid = node(&[property("name", b"\0", false)], &[]);
    let mut no_terminator = valid.clone();
    no_terminator[8..40].fill(b'n');
    assert_eq!(
        atomic_error(&no_terminator, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Runtime(flat_dt::FlatDtError::InvalidPropertyName)
    );
    let mut after_nul = valid.clone();
    after_nul[8 + 6] = b'x';
    assert!(matches!(
        atomic_error(&after_nul, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Firmware(_)
    ));
    let unnamed = node(&[property("other", b"\0", false)], &[]);
    assert_eq!(
        atomic_error(&unnamed, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Runtime(flat_dt::FlatDtError::InvalidNameProperty)
    );
    let child = node(&[property("name", b"duplicate\0", false)], &[]);
    let duplicate = node(&[property("name", b"\0", false)], &[child.clone(), child]);
    assert_eq!(
        atomic_error(&duplicate, &[], MAX_OUTPUT_BYTES, 0x4000, room()),
        Error::Runtime(flat_dt::FlatDtError::DuplicateChild)
    );
}

#[test]
fn successful_commit_changes_only_output_span_and_reads_declared_ram_values() {
    let source = fixture();
    let ids = template_ids(&source).unwrap();
    let mut ram = [0xa5; 2048];
    let actual = RamExtent::from_backing(0x4000, &ram).unwrap();
    let values = actual.little_endian_values();
    let bindings = [binding(ids[0], &values[0]), binding(ids[1], &values[1])];
    let plan = prepare(&source, &bindings, MAX_OUTPUT_BYTES).unwrap();
    plan.write_reserved(&mut ram, actual.base(), room())
        .unwrap();
    let offset = (room().address - actual.base()) as usize;
    assert!(ram[..offset].iter().all(|byte| *byte == 0xa5));
    assert!(ram[offset + plan.bytes().len()..]
        .iter()
        .all(|byte| *byte == 0xa5));
    for (name, expected) in [("dram-base", actual.base()), ("dram-size", actual.size())] {
        // Independent test reader locates the property header in committed RAM.
        let wire_name = property(name, &[], false);
        let property_offset = plan
            .bytes()
            .windows(32)
            .position(|bytes| bytes == &wire_name[..32])
            .unwrap();
        let start=offset+property_offset+36;
        let value=u64::from_le_bytes(ram[start..start+8].try_into().unwrap());
        assert_eq!(value, expected);
    }
}

#[test]
fn current_runtime_property_ceiling_is_preserved_explicitly() {
    for extra in [false, true] {
        let mut children = Vec::new();
        for child in 0..64 {
            let mut properties = vec![property(
                "name",
                format!("node-{child}\0").as_bytes(),
                false,
            )];
            let additional = if child == 63 && !extra { 62 } else { 63 };
            for p in 0..additional {
                properties.push(property(&format!("value-{p}"), &[1], false));
            }
            children.push(node(&properties, &[]));
        }
        let source = node(&[property("name", b"\0", false)], &children);
        assert_eq!(
            FirmwareDeviceTree::parse(&source)
                .unwrap()
                .statistics()
                .properties,
            if extra { 4097 } else { 4096 }
        );
        let result = prepare(&source, &[], MAX_OUTPUT_BYTES);
        if extra {
            assert!(matches!(
                result,
                Err(Error::Runtime(flat_dt::FlatDtError::LimitsExceeded))
            ));
        } else {
            assert!(result.is_ok());
        }
    }
}
