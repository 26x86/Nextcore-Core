use nextcore_core::selected_boot_input::{
    copy_selected_payload, parse_selected_boot_record, parse_selected_payload_record,
    verify_selected_image, ExpectedBootImage, VerifyError, MAX_SELECTED_IMAGE_BYTES,
};
use sha2::{Digest, Sha256};

#[test]
fn selection_is_required_and_bounded() {
    let bytes = b"authored selected image";
    assert_eq!(
        verify_selected_image(None, bytes).unwrap_err(),
        VerifyError::MissingSelection
    );
    assert_eq!(
        ExpectedBootImage::new(0, [0; 32]).unwrap_err(),
        VerifyError::InvalidSelection
    );
    assert_eq!(
        ExpectedBootImage::new(MAX_SELECTED_IMAGE_BYTES + 1, [0; 32]).unwrap_err(),
        VerifyError::InvalidSelection
    );
}

#[test]
fn companion_binds_signed_and_decoded_bytes_before_owned_copy() {
    let signed = b"authored signed container";
    let decoded = b"authored decoded instruction bytes";
    let hex = |bytes: &[u8]| -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    let selected_text = format!(
        "nextcore-selected-iboot-v1\n\
         device-class=j274ap\n\
         source-sha256={}\n\
         manifest-sha256={}\n\
         image-path=\\EFI\\NextCore\\selected.im4p\n\
         image-bytes={}\n\
         image-sha256={}\n",
        "a".repeat(64),
        "b".repeat(64),
        signed.len(),
        hex(signed)
    );
    let companion_text = format!(
        "nextcore-selected-iboot-payload-v1\n\
         signed-sha256={}\n\
         decoded-path=\\EFI\\NextCore\\selected.raw\n\
         decoded-bytes={}\n\
         decoded-sha256={}\n",
        hex(signed),
        decoded.len(),
        hex(decoded)
    );
    let selected = parse_selected_boot_record(selected_text.as_bytes()).unwrap();
    let companion = parse_selected_payload_record(companion_text.as_bytes()).unwrap();
    let mut owned = vec![0u8; decoded.len() + 8];
    assert_eq!(
        copy_selected_payload(&selected, signed, &companion, decoded, &mut owned).unwrap(),
        decoded.len()
    );
    assert_eq!(&owned[..decoded.len()], decoded);
    assert!(owned[decoded.len()..].iter().all(|byte| *byte == 0));

    let mut destination = vec![0xa5; decoded.len()];
    let mut changed_signed = signed.to_vec();
    changed_signed[0] ^= 1;
    assert_eq!(
        copy_selected_payload(
            &selected,
            &changed_signed,
            &companion,
            decoded,
            &mut destination
        ),
        Err(VerifyError::DigestMismatch)
    );
    let mut changed_decoded = decoded.to_vec();
    changed_decoded[0] ^= 1;
    assert_eq!(
        copy_selected_payload(
            &selected,
            signed,
            &companion,
            &changed_decoded,
            &mut destination
        ),
        Err(VerifyError::DigestMismatch)
    );
    let changed_binding = companion_text.replace(&hex(signed), &"0".repeat(64));
    let wrong_companion = parse_selected_payload_record(changed_binding.as_bytes()).unwrap();
    assert_eq!(
        copy_selected_payload(
            &selected,
            signed,
            &wrong_companion,
            decoded,
            &mut destination
        ),
        Err(VerifyError::BindingMismatch)
    );
    assert!(destination.iter().all(|byte| *byte == 0xa5));
    assert_eq!(
        copy_selected_payload(
            &selected,
            signed,
            &companion,
            decoded,
            &mut destination[..4]
        ),
        Err(VerifyError::DestinationCapacity)
    );
    assert_eq!(
        parse_selected_payload_record(companion_text.replace("selected.raw", "../raw").as_bytes()),
        Err(VerifyError::InvalidRecord)
    );
}

#[test]
fn exact_bytes_are_required_before_borrowed_use() {
    let owned = b"authored selected image".to_vec();
    let digest: [u8; 32] = Sha256::digest(&owned).into();
    let expected = ExpectedBootImage::new(owned.len(), digest).unwrap();
    let verified = verify_selected_image(Some(&expected), &owned).unwrap();
    assert_eq!(verified.bytes(), owned.as_slice());
    assert_eq!(verified.sha256(), digest);
    drop(verified);

    let mut changed = owned.clone();
    changed[0] ^= 1;
    assert_eq!(
        verify_selected_image(Some(&expected), &changed).unwrap_err(),
        VerifyError::DigestMismatch
    );
    assert_eq!(
        verify_selected_image(Some(&expected), &owned[..owned.len() - 1]).unwrap_err(),
        VerifyError::SizeMismatch
    );
}

#[test]
fn record_selects_authored_image_and_rejects_changed_claim() {
    let bytes = b"authored selected image";
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let record = format!(
        "nextcore-selected-iboot-v1\n\
         device-class=j274ap\n\
         source-sha256={}\n\
         manifest-sha256={}\n\
         image-path=\\EFI\\NextCore\\selected.im4p\n\
         image-bytes={}\n\
         image-sha256={hex}\n",
        "a".repeat(64),
        "b".repeat(64),
        bytes.len()
    );
    let selected = parse_selected_boot_record(record.as_bytes()).unwrap();
    assert_eq!(selected.device_class(), "j274ap");
    assert_eq!(selected.image_path(), r"\EFI\NextCore\selected.im4p");
    assert_eq!(selected.source_sha256(), [0xaa; 32]);
    assert_eq!(selected.manifest_sha256(), [0xbb; 32]);
    assert_eq!(
        verify_selected_image(Some(selected.expected()), bytes)
            .unwrap()
            .bytes(),
        bytes
    );
    assert_eq!(
        parse_selected_boot_record(record.replace("image-path=", "other-path=").as_bytes())
            .unwrap_err(),
        VerifyError::InvalidRecord
    );
    assert_eq!(
        parse_selected_boot_record(record.replace("j274ap", "..274ap").as_bytes()).unwrap_err(),
        VerifyError::InvalidRecord
    );
    assert_eq!(
        parse_selected_boot_record(
            record
                .replace(r"\EFI\NextCore\selected.im4p", r"\")
                .as_bytes()
        )
        .unwrap_err(),
        VerifyError::InvalidRecord
    );
    assert_eq!(
        parse_selected_boot_record(record.replace(&hex, &"A".repeat(64)).as_bytes()).unwrap_err(),
        VerifyError::InvalidRecord
    );
}
