use nextcore_core::{
    firmware_dt::{
        FirmwareDeviceTree, FirmwareDtError as E, PropertyKind, MAX_DEPTH, MAX_INPUT_BYTES,
        TEMPLATE_FLAG,
    },
    flat_dt::{self, FlatNode, FlatProperty},
};

fn property(name: &str, value: &[u8], template: bool) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    bytes.extend_from_slice(
        &((value.len() as u32) | if template { TEMPLATE_FLAG } else { 0 }).to_le_bytes(),
    );
    bytes.extend_from_slice(value);
    bytes.resize((bytes.len() + 3) & !3, 0);
    bytes
}
fn node(properties: &[Vec<u8>], children: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = (properties.len() as u32).to_le_bytes().to_vec();
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
        &[property("name", b"\0", false)],
        &[node(
            &[
                property("name", b"chosen\0", false),
                property("counter", b"provider.counter\0", true),
                property("literal", &[9, 8, 7], false),
            ],
            &[],
        )],
    )
}

#[test]
fn templates_and_literal_values_preserve_the_original_wire_bytes() {
    let bytes = fixture();
    let before = bytes.clone();
    let tree = FirmwareDeviceTree::parse(&bytes).unwrap();
    let stats = tree.statistics();
    assert_eq!(
        (
            stats.nodes,
            stats.properties,
            stats.templates,
            stats.maximum_depth
        ),
        (2, 4, 1, 1)
    );
    assert_eq!(tree.source().as_ptr(), bytes.as_ptr());
    let props = tree.root().children()[0].properties();
    let template = &props[1];
    assert_eq!(template.name(), "counter");
    assert_eq!(template.kind(), PropertyKind::Template);
    assert_eq!(template.raw_length(), TEMPLATE_FLAG | 17);
    assert_eq!(template.template_ascii_cstr(), Some("provider.counter"));
    assert_eq!(
        template.value(),
        &bytes[template.offset() + 36..template.offset() + 53]
    );
    assert_eq!(
        template.raw_name(),
        &bytes[template.offset()..template.offset() + 32]
    );
    assert_eq!(props[2].kind(), PropertyKind::Literal);
    assert_eq!(props[2].value(), [9, 8, 7]);
    assert_eq!(
        tree.validated_runtime_bytes(),
        Err(E::UnresolvedTemplates { count: 1 })
    );
    assert!(flat_dt::validate(&bytes).is_err());
    assert_eq!(bytes, before);
}

#[test]
fn opaque_or_malformed_template_bodies_remain_unresolved() {
    for body in [&b"unterminated"[..], &[0xff, 0], &[], b"a\0b\0"] {
        let bytes = node(
            &[
                property("name", b"\0", false),
                property("input", body, true),
            ],
            &[],
        );
        let tree = FirmwareDeviceTree::parse(&bytes).unwrap();
        let prop = &tree.root().properties()[1];
        assert_eq!(prop.kind(), PropertyKind::Template);
        assert_eq!(prop.value(), body);
        assert_eq!(prop.template_ascii_cstr(), None);
        assert_eq!(
            tree.validated_runtime_bytes(),
            Err(E::UnresolvedTemplates { count: 1 })
        );
    }
}

#[test]
fn unmodified_runtime_data_must_still_pass_the_existing_strict_validator() {
    let bytes = flat_dt::encode(&FlatNode {
        name: "".into(),
        properties: vec![FlatProperty {
            name: "literal".into(),
            value: vec![1],
        }],
        children: vec![],
    })
    .unwrap();
    let tree = FirmwareDeviceTree::parse(&bytes).unwrap();
    assert_eq!(tree.validated_runtime_bytes().unwrap(), bytes);
    let mut padded = bytes.clone();
    *padded.last_mut().unwrap() = 0x7a;
    let tree = FirmwareDeviceTree::parse(&padded).unwrap();
    assert_eq!(tree.root().properties()[1].padding(), [0, 0, 0x7a]);
    assert!(matches!(tree.validated_runtime_bytes(), Err(E::Runtime(_))));
    assert_eq!(tree.source(), padded);
}

#[test]
fn every_truncation_and_trailing_byte_is_rejected() {
    let mut bytes = fixture();
    for end in 0..bytes.len() {
        assert!(
            FirmwareDeviceTree::parse(&bytes[..end]).is_err(),
            "accepted prefix {end}"
        );
    }
    bytes.push(0);
    assert!(matches!(
        FirmwareDeviceTree::parse(&bytes),
        Err(E::TrailingData)
    ));
}

#[test]
fn untrusted_counts_and_flagged_lengths_are_bounded_before_payload_access() {
    for word in [0, 4] {
        let mut bytes = vec![0; 8];
        bytes[word..word + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            FirmwareDeviceTree::parse(&bytes),
            Err(E::LimitsExceeded)
        ));
    }
    let mut bytes = node(&[property("large", &[], true)], &[]);
    bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        FirmwareDeviceTree::parse(&bytes),
        Err(E::Truncated)
    ));
    assert!(matches!(
        FirmwareDeviceTree::parse(&vec![0; MAX_INPUT_BYTES + 1]),
        Err(E::InputTooLarge)
    ));
}

#[test]
fn names_and_recursion_depth_have_independent_limits() {
    for name in [vec![0; 32], vec![0xff; 32], {
        let mut name = vec![0; 32];
        name[0] = b'a';
        name[2] = b'b';
        name
    }] {
        let mut bytes = node(&[property("valid", &[], false)], &[]);
        bytes[8..40].copy_from_slice(&name);
        assert!(matches!(
            FirmwareDeviceTree::parse(&bytes),
            Err(E::InvalidPropertyName)
        ));
    }
    let mut bytes = node(&[], &[]);
    for _ in 0..MAX_DEPTH {
        bytes = node(&[], &[bytes]);
    }
    assert_eq!(
        FirmwareDeviceTree::parse(&bytes)
            .unwrap()
            .statistics()
            .maximum_depth,
        MAX_DEPTH
    );
    bytes = node(&[], &[bytes]);
    assert!(matches!(
        FirmwareDeviceTree::parse(&bytes),
        Err(E::LimitsExceeded)
    ));
}

#[test]
fn aggregate_node_and_property_budgets_cover_wide_trees() {
    let leaf = node(&[], &[]);
    let branch = node(&[], &[leaf.clone(), leaf.clone(), leaf.clone()]);
    let mut children = vec![branch.clone(); 1023];
    children.push(node(&[], &[leaf.clone(), leaf]));
    let bytes = node(&[], &children);
    assert_eq!(
        FirmwareDeviceTree::parse(&bytes)
            .unwrap()
            .statistics()
            .nodes,
        4096
    );
    let bytes = node(&[], &vec![branch; 1024]);
    assert!(matches!(
        FirmwareDeviceTree::parse(&bytes),
        Err(E::LimitsExceeded)
    ));
    let branch = node(&vec![property("value", &[], false); 1024], &[]);
    let bytes = node(&[], &vec![branch.clone(); 16]);
    assert_eq!(
        FirmwareDeviceTree::parse(&bytes)
            .unwrap()
            .statistics()
            .properties,
        16384
    );
    let bytes = node(&[], &vec![branch; 17]);
    assert!(bytes.len() < MAX_INPUT_BYTES);
    assert!(matches!(
        FirmwareDeviceTree::parse(&bytes),
        Err(E::LimitsExceeded)
    ));
}
