use nextcore_core::selected_boot_input::{
    parse_selected_boot_record, parse_selected_bootkc_record, verify_selected_image,
    verify_selected_record_digest, VerifyError,
};
use sha2::{Digest, Sha256};

fn digest(data: &[u8]) -> String {
    let bytes: [u8; 32] = Sha256::digest(data).into();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn selected() -> Vec<u8> {
    format!(
        "nextcore-selected-iboot-v1\ndevice-class=j274ap\nsource-sha256={}\nmanifest-sha256={}\nimage-path=\\EFI\\NextCore\\iboot.im4p\nimage-bytes=4\nimage-sha256={}\n",
        "11".repeat(32), "22".repeat(32), digest(b"boot")
    ).into_bytes()
}

fn bootkc(signed: &[u8], decoded: &[u8]) -> Vec<u8> {
    format!(
        "nextcore-selected-bootkc-v1\ndevice-class=j274ap\nsource-sha256={}\nmanifest-sha256={}\nsigned-path=\\EFI\\NextCore\\bootkc.im4p\nsigned-bytes={}\nsigned-sha256={}\ndecoded-path=\\EFI\\NextCore\\bootkc.raw\ndecoded-bytes={}\ndecoded-sha256={}\n",
        "11".repeat(32), "22".repeat(32), signed.len(), digest(signed), decoded.len(), digest(decoded)
    ).into_bytes()
}

#[test]
fn selected_bootkc_binds_identity_and_both_exact_inputs() {
    let selected = parse_selected_boot_record(&selected()).unwrap();
    let signed = b"signed";
    let decoded = b"decoded";
    let record = parse_selected_bootkc_record(&bootkc(signed, decoded)).unwrap();
    assert!(record.binds_selected(&selected));
    assert!(record.matches_selected_path(&selected, r"\EFI\NextCore\bootkc.raw"));
    assert!(!record.matches_selected_path(&selected, r"\EFI\NextCore\other.raw"));
    assert_eq!(record.decoded_path(), r"\EFI\NextCore\bootkc.raw");
    assert_eq!(
        verify_selected_image(Some(record.signed()), signed)
            .unwrap()
            .bytes(),
        signed
    );
    assert_eq!(
        verify_selected_image(Some(record.decoded()), decoded)
            .unwrap()
            .bytes(),
        decoded
    );
    assert_eq!(
        verify_selected_image(Some(record.signed()), b"signeD").unwrap_err(),
        VerifyError::DigestMismatch
    );
    assert_eq!(
        verify_selected_image(Some(record.decoded()), b"decodeD").unwrap_err(),
        VerifyError::DigestMismatch
    );

    let changed_identity = String::from_utf8(bootkc(signed, decoded))
        .unwrap()
        .replace("device-class=j274ap", "device-class=j274bp");
    let changed = parse_selected_bootkc_record(changed_identity.as_bytes()).unwrap();
    assert!(!changed.binds_selected(&selected));

    let source_needle = format!("source-sha256={}", "11".repeat(32));
    let source_replacement = format!("source-sha256={}", "33".repeat(32));
    let changed_source = String::from_utf8(bootkc(signed, decoded))
        .unwrap()
        .replace(source_needle.as_str(), source_replacement.as_str());
    let changed = parse_selected_bootkc_record(changed_source.as_bytes()).unwrap();
    assert!(!changed.binds_selected(&selected));

    let manifest_needle = format!("manifest-sha256={}", "22".repeat(32));
    let manifest_replacement = format!("manifest-sha256={}", "44".repeat(32));
    let changed_manifest = String::from_utf8(bootkc(signed, decoded))
        .unwrap()
        .replace(manifest_needle.as_str(), manifest_replacement.as_str());
    let changed = parse_selected_bootkc_record(changed_manifest.as_bytes()).unwrap();
    assert!(!changed.binds_selected(&selected));
}

#[test]
fn selected_bootkc_rejects_overlapping_paths_and_oversized_decoded_claim() {
    let original = String::from_utf8(bootkc(b"signed", b"decoded")).unwrap();
    let overlapping = original.replace(
        "decoded-path=\\EFI\\NextCore\\bootkc.raw",
        "decoded-path=\\EFI\\NextCore\\bootkc.im4p",
    );
    assert_eq!(
        parse_selected_bootkc_record(overlapping.as_bytes()).unwrap_err(),
        VerifyError::InvalidRecord
    );
    let oversized = original.replace("decoded-bytes=7", "decoded-bytes=134217729");
    assert_eq!(
        parse_selected_bootkc_record(oversized.as_bytes()).unwrap_err(),
        VerifyError::InvalidSelection
    );

    let actual_target_sizes = original
        .replace("signed-bytes=6", "signed-bytes=32727616")
        .replace("decoded-bytes=7", "decoded-bytes=122945536");
    let record = parse_selected_bootkc_record(actual_target_sizes.as_bytes()).unwrap();
    assert_eq!(record.signed().byte_len(), 32_727_616);
    assert_eq!(record.decoded().byte_len(), 122_945_536);
}

#[test]
fn selected_bootkc_seal_rejects_a_self_consistent_replacement() {
    let selected = parse_selected_boot_record(&selected()).unwrap();
    let signed = b"signed";
    let original_record = bootkc(signed, b"decoded");
    let expected_digest: [u8; 32] = Sha256::digest(&original_record).into();
    assert_eq!(
        verify_selected_record_digest(None, &original_record),
        Err(VerifyError::MissingSelection)
    );
    verify_selected_record_digest(Some(expected_digest), &original_record).unwrap();

    let changed_decoded = b"replaced";
    let changed_record = bootkc(signed, changed_decoded);
    let changed = parse_selected_bootkc_record(&changed_record).unwrap();
    assert!(changed.matches_selected_path(&selected, changed.decoded_path()));
    verify_selected_image(Some(changed.signed()), signed).unwrap();
    verify_selected_image(Some(changed.decoded()), changed_decoded).unwrap();
    assert_eq!(
        verify_selected_record_digest(Some(expected_digest), &changed_record),
        Err(VerifyError::DigestMismatch)
    );
}
