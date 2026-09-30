use nextcore_core::selected_boot_input::{
    parse_selected_boot_record, parse_selected_startup_companions, verify_selected_image,
    verify_selected_record_digest, VerifyError,
};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn selected() -> Vec<u8> {
    format!(
        "nextcore-selected-iboot-v1\ndevice-class=j274ap\nsource-sha256={}\nmanifest-sha256={}\nimage-path=\\EFI\\NextCore\\selected.im4p\nimage-bytes=4\nimage-sha256={}\n",
        "11".repeat(32), "22".repeat(32), hex(b"boot")
    ).into_bytes()
}

fn record(sptm: &[u8], txm: &[u8]) -> Vec<u8> {
    format!(
        "nextcore-selected-startup-companions-v1\ndevice-class=j274ap\nsource-sha256={}\nmanifest-sha256={}\nsptm-signed-path=\\EFI\\NextCore\\sptm.im4p\nsptm-signed-bytes=4\nsptm-signed-sha256={}\nsptm-decoded-path=\\EFI\\NextCore\\sptm.raw\nsptm-decoded-bytes={}\nsptm-decoded-sha256={}\ntxm-signed-path=\\EFI\\NextCore\\txm.im4p\ntxm-signed-bytes=4\ntxm-signed-sha256={}\ntxm-decoded-path=\\EFI\\NextCore\\txm.raw\ntxm-decoded-bytes={}\ntxm-decoded-sha256={}\n",
        "11".repeat(32), "22".repeat(32), hex(b"sptm"), sptm.len(), hex(sptm),
        hex(b"txm!"), txm.len(), hex(txm)
    ).into_bytes()
}

#[test]
fn selected_companions_bind_identity_and_exact_role_bytes() {
    let selected = parse_selected_boot_record(&selected()).unwrap();
    let record = parse_selected_startup_companions(&record(b"raw sptm", b"raw txm")).unwrap();
    assert!(record.binds_selected(&selected));
    assert!(record.paths_disjoint_from(&[selected.image_path()]));
    assert!(!record.paths_disjoint_from(&[r"\efi\nextcore\SPTM.IM4P"]));
    for (role, signed, decoded) in [
        (record.sptm(), b"sptm".as_slice(), b"raw sptm".as_slice()),
        (record.txm(), b"txm!".as_slice(), b"raw txm".as_slice()),
    ] {
        assert_eq!(
            verify_selected_image(Some(role.signed()), signed)
                .unwrap()
                .bytes(),
            signed
        );
        assert_eq!(
            verify_selected_image(Some(role.decoded()), decoded)
                .unwrap()
                .bytes(),
            decoded
        );
        let mut changed = signed.to_vec();
        changed[0] ^= 1;
        assert_eq!(
            verify_selected_image(Some(role.signed()), &changed).unwrap_err(),
            VerifyError::DigestMismatch
        );
        let mut changed = decoded.to_vec();
        changed[0] ^= 1;
        assert_eq!(
            verify_selected_image(Some(role.decoded()), &changed).unwrap_err(),
            VerifyError::DigestMismatch
        );
    }
}

#[test]
fn seal_rejects_self_consistent_record_replacement_before_file_admission() {
    let original = record(b"raw sptm", b"raw txm");
    let seal: [u8; 32] = Sha256::digest(&original).into();
    verify_selected_record_digest(Some(seal), &original).unwrap();
    let changed = record(b"new sptm", b"raw txm");
    let parsed = parse_selected_startup_companions(&changed).unwrap();
    verify_selected_image(Some(parsed.sptm().decoded()), b"new sptm").unwrap();
    assert_eq!(
        verify_selected_record_digest(Some(seal), &changed),
        Err(VerifyError::DigestMismatch)
    );
    assert_eq!(
        verify_selected_record_digest(None, &original),
        Err(VerifyError::MissingSelection)
    );
}

#[test]
fn fixed_roles_paths_and_identity_are_required() {
    let original = String::from_utf8(record(b"raw sptm", b"raw txm")).unwrap();
    let changed = original.replace("device-class=j274ap", "device-class=j274bp");
    assert!(!parse_selected_startup_companions(changed.as_bytes())
        .unwrap()
        .binds_selected(&parse_selected_boot_record(&selected()).unwrap()));
    let changed = original.replace("txm-signed-path=", "sptm-signed-path=");
    assert_eq!(
        parse_selected_startup_companions(changed.as_bytes()),
        Err(VerifyError::InvalidRecord)
    );
    let changed = original.replace("\\EFI\\NextCore\\txm.raw", "\\EFI\\NextCore\\sptm.raw");
    assert_eq!(
        parse_selected_startup_companions(changed.as_bytes()),
        Err(VerifyError::InvalidRecord)
    );
    let changed = original.replace("\\EFI\\NextCore\\txm.raw", "\\efi\\nextcore\\SPTM.RAW");
    assert_eq!(
        parse_selected_startup_companions(changed.as_bytes()),
        Err(VerifyError::InvalidRecord)
    );
    let changed = original.replace("\\EFI\\NextCore\\txm.raw", "\\EFI\\..\\txm.raw");
    assert_eq!(
        parse_selected_startup_companions(changed.as_bytes()),
        Err(VerifyError::InvalidRecord)
    );
    let changed = original.replace("sptm-decoded-bytes=8", "sptm-decoded-bytes=67108865");
    assert_eq!(
        parse_selected_startup_companions(changed.as_bytes()),
        Err(VerifyError::InvalidSelection)
    );
}
