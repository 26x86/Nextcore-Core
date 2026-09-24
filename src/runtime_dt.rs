//! Materialize explicit provider values into a checked runtime DeviceTree.
//! No expression interpreter, platform defaults, allocator or execution right.

use alloc::{string::String, vec::Vec};
use firmware_dt::{FirmwareDeviceTree, FirmwareNode, PropertyKind};
use crate::{firmware_dt, flat_dt};
use sha2::{Digest, Sha256};

pub const MAX_OUTPUT_BYTES: usize = flat_dt::MAX_SERIALIZED_SIZE;
pub const MAX_BINDINGS: usize = flat_dt::MAX_PROPERTIES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyId {
    pub source_sha256: [u8; 32],
    pub property_offset: usize,
}

/// Caller-supplied bytes and provenance label. A label is not authentication.
pub struct ProvidedValue<'a> {
    pub property: PropertyId,
    pub provider: &'a str,
    pub value: &'a [u8],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Firmware(firmware_dt::FirmwareDtError),
    Runtime(flat_dt::FlatDtError),
    Limits,
    Allocation,
    SourceIdentity,
    StructuralTemplate,
    InvalidProvider,
    DuplicateBinding,
    UnknownBinding,
    MissingProviders(Vec<PropertyId>),
    ArithmeticOverflow,
    InvalidRam,
    InvalidReservation,
    DestinationCapacity,
}

pub fn source_identity(source: &[u8]) -> [u8; 32] {
    Sha256::digest(source).into()
}

fn collect_templates(
    node: &FirmwareNode<'_>,
    digest: [u8; 32],
    ids: &mut Vec<PropertyId>,
) -> Result<(), Error> {
    for property in node.properties() {
        if property.kind() == PropertyKind::Template {
            if property.name() == "name" {
                return Err(Error::StructuralTemplate);
            }
            if ids.len() == MAX_BINDINGS {
                return Err(Error::Limits);
            }
            ids.try_reserve(1).map_err(|_| Error::Allocation)?;
            ids.push(PropertyId {
                source_sha256: digest,
                property_offset: property.offset(),
            });
        }
    }
    for child in node.children() {
        collect_templates(child, digest, ids)?;
    }
    Ok(())
}

/// Stable within the exact source digest; no template expression is returned.
pub fn template_ids(source: &[u8]) -> Result<Vec<PropertyId>, Error> {
    let tree = FirmwareDeviceTree::parse(source).map_err(Error::Firmware)?;
    let mut ids = Vec::new();
    collect_templates(tree.root(), source_identity(source), &mut ids)?;
    Ok(ids)
}

/// Checked wire measurement without allocating the corresponding payload.
pub fn property_wire_bytes(value_length: u64) -> Result<u64, Error> {
    let padded = value_length
        .checked_add(3)
        .ok_or(Error::ArithmeticOverflow)?
        & !3;
    let total = padded.checked_add(36).ok_or(Error::ArithmeticOverflow)?;
    if value_length >= u64::from(firmware_dt::TEMPLATE_FLAG) {
        return Err(Error::Limits);
    }
    Ok(total)
}

fn replacement<'a, 'b>(
    offset: usize,
    provided: &'a [ProvidedValue<'b>],
) -> Option<&'a ProvidedValue<'b>> {
    provided
        .iter()
        .find(|binding| binding.property.property_offset == offset)
}

fn measure(
    node: &FirmwareNode<'_>,
    provided: &[ProvidedValue<'_>],
    total: &mut u64,
) -> Result<(), Error> {
    *total = total.checked_add(8).ok_or(Error::ArithmeticOverflow)?;
    for property in node.properties() {
        let length = match property.kind() {
            PropertyKind::Literal => property.value().len(),
            PropertyKind::Template => replacement(property.offset(), provided)
                .ok_or(Error::UnknownBinding)?
                .value
                .len(),
        };
        *total = total
            .checked_add(property_wire_bytes(length as u64)?)
            .ok_or(Error::ArithmeticOverflow)?;
    }
    for child in node.children() {
        measure(child, provided, total)?;
    }
    Ok(())
}

fn write(node: &FirmwareNode<'_>, provided: &[ProvidedValue<'_>], output: &mut Vec<u8>) {
    output.extend_from_slice(&(node.properties().len() as u32).to_le_bytes());
    output.extend_from_slice(&(node.children().len() as u32).to_le_bytes());
    for property in node.properties() {
        output.extend_from_slice(property.raw_name());
        if property.kind() == PropertyKind::Literal {
            output.extend_from_slice(&property.raw_length().to_le_bytes());
            output.extend_from_slice(property.value());
            output.extend_from_slice(property.padding());
        } else {
            // All bindings and measurements were checked before buffer allocation.
            let value = replacement(property.offset(), provided)
                .expect("preflighted binding")
                .value;
            output.extend_from_slice(&(value.len() as u32).to_le_bytes());
            output.extend_from_slice(value);
            output.resize(output.len() + ((4 - value.len() % 4) % 4), 0);
        }
    }
    for child in node.children() {
        write(child, provided, output);
    }
}

#[derive(Debug)]
pub struct AppliedBinding {
    pub property: PropertyId,
    pub provider: String,
    pub value_sha256: [u8; 32],
    pub value_bytes: usize,
}

#[derive(Debug)]
pub struct MaterializedTree<'a> {
    source: &'a [u8],
    bytes: Vec<u8>,
    source_sha256: [u8; 32],
    replacement_count: usize,
    bindings: Vec<AppliedBinding>,
}

impl MaterializedTree<'_> {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn source(&self) -> &[u8] {
        self.source
    }
    pub fn source_sha256(&self) -> [u8; 32] {
        self.source_sha256
    }
    pub fn replacement_count(&self) -> usize {
        self.replacement_count
    }
    pub fn bindings(&self) -> &[AppliedBinding] {
        &self.bindings
    }

    /// A full preflight precedes the sole copy. The reservation is a caller
    /// assertion; this method does not establish allocator ownership or MMU state.
    pub fn write_reserved(
        &self,
        ram: &mut [u8],
        base: u64,
        reservation: DeclaredReservation,
    ) -> Result<(), Error> {
        let observed = RamExtent::from_backing(base, ram)?;
        let end = reservation
            .address
            .checked_add(reservation.capacity)
            .ok_or(Error::ArithmeticOverflow)?;
        if reservation.capacity == 0
            || reservation.address % 4 != 0
            || reservation.address < observed.base
            || end > observed.end()
        {
            return Err(Error::InvalidReservation);
        }
        if self.bytes.len() as u64 > reservation.capacity {
            return Err(Error::DestinationCapacity);
        }
        let offset =
            usize::try_from(reservation.address - base).map_err(|_| Error::ArithmeticOverflow)?;
        let output_end = offset
            .checked_add(self.bytes.len())
            .ok_or(Error::ArithmeticOverflow)?;
        if output_end > ram.len() {
            return Err(Error::DestinationCapacity);
        }
        ram[offset..output_end].copy_from_slice(&self.bytes);
        Ok(())
    }
}

pub fn prepare<'a>(
    source: &'a [u8],
    provided: &[ProvidedValue<'_>],
    maximum_output: usize,
) -> Result<MaterializedTree<'a>, Error> {
    if maximum_output == 0 || maximum_output > MAX_OUTPUT_BYTES || provided.len() > MAX_BINDINGS {
        return Err(Error::Limits);
    }
    let tree = FirmwareDeviceTree::parse(source).map_err(Error::Firmware)?;
    let digest = source_identity(source);
    let mut templates = Vec::new();
    collect_templates(tree.root(), digest, &mut templates)?;
    for (index, binding) in provided.iter().enumerate() {
        if binding.property.source_sha256 != digest {
            return Err(Error::SourceIdentity);
        }
        if binding.provider.is_empty()
            || binding.provider.len() > 64
            || !binding
                .provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        {
            return Err(Error::InvalidProvider);
        }
        if provided[..index]
            .iter()
            .any(|p| p.property == binding.property)
        {
            return Err(Error::DuplicateBinding);
        }
        if !templates.contains(&binding.property) {
            return Err(Error::UnknownBinding);
        }
    }
    let mut missing = Vec::new();
    missing
        .try_reserve_exact(templates.len())
        .map_err(|_| Error::Allocation)?;
    for id in &templates {
        if replacement(id.property_offset, provided).is_none() {
            missing.push(*id);
        }
    }
    if !missing.is_empty() {
        return Err(Error::MissingProviders(missing));
    }
    let mut length = 0;
    measure(tree.root(), provided, &mut length)?;
    if length > maximum_output as u64 {
        return Err(Error::Limits);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize)
        .map_err(|_| Error::Allocation)?;
    write(tree.root(), provided, &mut bytes);
    debug_assert_eq!(bytes.len(), length as usize);
    flat_dt::validate(&bytes).map_err(Error::Runtime)?;
    let mut bindings = Vec::new();
    bindings
        .try_reserve_exact(provided.len())
        .map_err(|_| Error::Allocation)?;
    for binding in provided {
        let mut provider = String::new();
        provider
            .try_reserve_exact(binding.provider.len())
            .map_err(|_| Error::Allocation)?;
        provider.push_str(binding.provider);
        bindings.push(AppliedBinding {
            property: binding.property,
            provider,
            value_sha256: source_identity(binding.value),
            value_bytes: binding.value.len(),
        });
    }
    Ok(MaterializedTree {
        source,
        bytes,
        source_sha256: digest,
        replacement_count: templates.len(),
        bindings,
    })
}

/// A caller-declared guest mapping base plus an observed backing byte length.
/// The base is not an observed host PA. No reserved-page map, allocator epoch,
/// owner, or continuing borrow is conveyed by this copyable description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RamExtent {
    base: u64,
    size: u64,
}
impl RamExtent {
    pub fn from_backing(base: u64, bytes: &[u8]) -> Result<Self, Error> {
        let size = u64::try_from(bytes.len()).map_err(|_| Error::ArithmeticOverflow)?;
        if size == 0 {
            return Err(Error::InvalidRam);
        }
        base.checked_add(size).ok_or(Error::ArithmeticOverflow)?;
        Ok(Self { base, size })
    }
    pub fn base(&self) -> u64 {
        self.base
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    fn end(&self) -> u64 {
        self.base + self.size
    }
    pub fn little_endian_values(&self) -> [[u8; 8]; 2] {
        [self.base.to_le_bytes(), self.size.to_le_bytes()]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeclaredReservation {
    pub address: u64,
    pub capacity: u64,
}

/// Atomic with respect to all returned errors; neither input nor destination is
/// changed until the complete output and reservation have passed validation.
pub fn materialize_into(
    source: &[u8],
    provided: &[ProvidedValue<'_>],
    maximum_output: usize,
    ram: &mut [u8],
    base: u64,
    reservation: DeclaredReservation,
) -> Result<usize, Error> {
    let tree = prepare(source, provided, maximum_output)?;
    tree.write_reserved(ram, base, reservation)?;
    Ok(tree.bytes().len())
}
