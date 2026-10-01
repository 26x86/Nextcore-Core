//! Exact byte gate for an explicitly selected boot image.
//! Caller-supplied expectations are content checks, not selection provenance.

use alloc::string::String;
use sha2::{Digest, Sha256};

/// Initial bound for a single firmware-read image before guest placement.
pub const MAX_SELECTED_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SELECTED_RECORD_BYTES: usize = 2048;
pub const MAX_SELECTED_PAYLOAD_RECORD_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    MissingSelection,
    InvalidSelection,
    SizeMismatch,
    DigestMismatch,
    InvalidRecord,
    BindingMismatch,
    DestinationCapacity,
}

/// Metadata emitted from a separately prepared selection; the record is not a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedBootRecord {
    device_class: String,
    source_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    image_path: String,
    expected: ExpectedBootImage,
}

/// Preparation metadata for a decoded derivative; neither record is a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPayloadRecord {
    signed_sha256: [u8; 32],
    decoded_path: String,
    expected: ExpectedBootImage,
}

impl SelectedPayloadRecord {
    pub fn decoded_path(&self) -> &str {
        &self.decoded_path
    }

    pub fn expected(&self) -> &ExpectedBootImage {
        &self.expected
    }
}

pub fn parse_selected_payload_record(input: &[u8]) -> Result<SelectedPayloadRecord, VerifyError> {
    if input.is_empty()
        || input.len() > MAX_SELECTED_PAYLOAD_RECORD_BYTES
        || !input.ends_with(b"\n")
    {
        return Err(VerifyError::InvalidRecord);
    }
    let text = core::str::from_utf8(input).map_err(|_| VerifyError::InvalidRecord)?;
    let mut lines = text.lines();
    if lines.next() != Some("nextcore-selected-iboot-payload-v1") {
        return Err(VerifyError::InvalidRecord);
    }
    let signed_sha256 = hex_digest(field(&mut lines, "signed-sha256=")?)?;
    let decoded_path = field(&mut lines, "decoded-path=")?;
    if !valid_efi_path(decoded_path) {
        return Err(VerifyError::InvalidRecord);
    }
    let byte_len = field(&mut lines, "decoded-bytes=")?
        .parse::<usize>()
        .map_err(|_| VerifyError::InvalidRecord)?;
    let sha256 = hex_digest(field(&mut lines, "decoded-sha256=")?)?;
    if lines.next().is_some() {
        return Err(VerifyError::InvalidRecord);
    }
    Ok(SelectedPayloadRecord {
        signed_sha256,
        decoded_path: String::from(decoded_path),
        expected: ExpectedBootImage::new(byte_len, sha256)?,
    })
}

/// Copy checked bytes into separately owned storage without choosing a runtime address.
pub fn copy_selected_payload(
    selected: &SelectedBootRecord,
    signed: &[u8],
    companion: &SelectedPayloadRecord,
    decoded: &[u8],
    destination: &mut [u8],
) -> Result<usize, VerifyError> {
    verify_selected_image(Some(selected.expected()), signed)?;
    if companion.signed_sha256 != selected.expected().sha256()
        || companion.decoded_path == selected.image_path()
    {
        return Err(VerifyError::BindingMismatch);
    }
    let verified = verify_selected_image(Some(companion.expected()), decoded)?;
    if destination.len() < verified.bytes().len() {
        return Err(VerifyError::DestinationCapacity);
    }
    destination[..verified.bytes().len()].copy_from_slice(verified.bytes());
    Ok(verified.bytes().len())
}

impl SelectedBootRecord {
    pub fn device_class(&self) -> &str {
        &self.device_class
    }

    pub fn source_sha256(&self) -> [u8; 32] {
        self.source_sha256
    }

    pub fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    pub fn image_path(&self) -> &str {
        &self.image_path
    }

    pub fn expected(&self) -> &ExpectedBootImage {
        &self.expected
    }
}

/// Parse the fixed-order, newline-terminated EFI metadata record.
pub fn parse_selected_boot_record(input: &[u8]) -> Result<SelectedBootRecord, VerifyError> {
    if input.is_empty() || input.len() > MAX_SELECTED_RECORD_BYTES || !input.ends_with(b"\n") {
        return Err(VerifyError::InvalidRecord);
    }
    let text = core::str::from_utf8(input).map_err(|_| VerifyError::InvalidRecord)?;
    let mut lines = text.lines();
    if lines.next() != Some("nextcore-selected-iboot-v1") {
        return Err(VerifyError::InvalidRecord);
    }
    let device_class = field(&mut lines, "device-class=")?;
    if device_class.is_empty()
        || device_class.len() > 64
        || !device_class
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(VerifyError::InvalidRecord);
    }
    let source_sha256 = hex_digest(field(&mut lines, "source-sha256=")?)?;
    let manifest_sha256 = hex_digest(field(&mut lines, "manifest-sha256=")?)?;
    let image_path = field(&mut lines, "image-path=")?;
    if !valid_efi_path(image_path) {
        return Err(VerifyError::InvalidRecord);
    }
    let byte_len = field(&mut lines, "image-bytes=")?
        .parse::<usize>()
        .map_err(|_| VerifyError::InvalidRecord)?;
    let sha256 = hex_digest(field(&mut lines, "image-sha256=")?)?;
    if lines.next().is_some() {
        return Err(VerifyError::InvalidRecord);
    }
    Ok(SelectedBootRecord {
        device_class: String::from(device_class),
        source_sha256,
        manifest_sha256,
        image_path: String::from(image_path),
        expected: ExpectedBootImage::new(byte_len, sha256)?,
    })
}

fn valid_efi_path(path: &str) -> bool {
    path.len() >= 2
        && path.len() <= 1024
        && path.starts_with('\\')
        && !path.contains("..")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'\\' | b'.' | b'_' | b'-'))
}

fn field<'a>(lines: &mut impl Iterator<Item = &'a str>, key: &str) -> Result<&'a str, VerifyError> {
    lines
        .next()
        .and_then(|line| line.strip_prefix(key))
        .ok_or(VerifyError::InvalidRecord)
}

fn hex_digest(text: &str) -> Result<[u8; 32], VerifyError> {
    if text.len() != 64 {
        return Err(VerifyError::InvalidRecord);
    }
    let mut out = [0; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or(VerifyError::InvalidRecord)?;
        let low = hex_nibble(pair[1]).ok_or(VerifyError::InvalidRecord)?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Exact content expectation supplied by a separately checked selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedBootImage {
    byte_len: usize,
    sha256: [u8; 32],
}

impl ExpectedBootImage {
    pub fn new(byte_len: usize, sha256: [u8; 32]) -> Result<Self, VerifyError> {
        if byte_len == 0 || byte_len > MAX_SELECTED_IMAGE_BYTES {
            return Err(VerifyError::InvalidSelection);
        }
        Ok(Self { byte_len, sha256 })
    }

    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// A byte view tied to the source buffer's lifetime after exact verification.
#[derive(Debug)]
pub struct VerifiedBootImage<'a> {
    bytes: &'a [u8],
    sha256: [u8; 32],
}

impl VerifiedBootImage<'_> {
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

pub fn verify_selected_image<'a>(
    selection: Option<&ExpectedBootImage>,
    source: &'a [u8],
) -> Result<VerifiedBootImage<'a>, VerifyError> {
    let selection = selection.ok_or(VerifyError::MissingSelection)?;
    if source.len() != selection.byte_len {
        return Err(VerifyError::SizeMismatch);
    }
    let measured: [u8; 32] = Sha256::digest(source).into();
    if measured != selection.sha256 {
        return Err(VerifyError::DigestMismatch);
    }
    Ok(VerifiedBootImage {
        bytes: source,
        sha256: measured,
    })
}
