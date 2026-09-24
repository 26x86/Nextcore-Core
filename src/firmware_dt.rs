//! Borrowed structural view of firmware device-tree properties and templates.
//!
//! Independently expressed from the public framing and template distinction in
//! <https://github.com/AsahiLinux/m1n1/blob/main/proxyclient/m1n1/adt.py>.
//! This is separate from the XNU runtime format validated by `flat_dt`:
//! <https://github.com/apple-oss-distributions/xnu/blob/main/pexpert/gen/device_tree.c>.
//! Template expressions remain opaque; no provider values are evaluated here.

use alloc::vec::Vec;
use core::fmt;

pub const TEMPLATE_FLAG: u32 = 1 << 31;
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_NODES: usize = 4096;
pub const MAX_PROPERTIES: usize = 16384;
pub const MAX_PROPERTIES_PER_NODE: usize = 1024;
pub const MAX_CHILDREN_PER_NODE: usize = 1024;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    Literal,
    Template,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FirmwareDtStatistics {
    pub nodes: usize,
    pub properties: usize,
    pub templates: usize,
    pub maximum_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirmwareDtError {
    InputTooLarge,
    Truncated,
    LimitsExceeded,
    InvalidPropertyName,
    TrailingData,
    AllocationFailed,
    UnresolvedTemplates { count: usize },
    Runtime(crate::flat_dt::FlatDtError),
}
impl fmt::Display for FirmwareDtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FIRMWARE_DT_{self:?}")
    }
}
impl core::error::Error for FirmwareDtError {}

#[derive(Debug)]
pub struct FirmwareProperty<'a> {
    offset: usize,
    raw_name: &'a [u8; 32],
    name: &'a str,
    raw_length: u32,
    value: &'a [u8],
    padding: &'a [u8],
}
impl<'a> FirmwareProperty<'a> {
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn raw_name(&self) -> &'a [u8; 32] {
        self.raw_name
    }
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn raw_length(&self) -> u32 {
        self.raw_length
    }
    pub fn kind(&self) -> PropertyKind {
        if self.raw_length & TEMPLATE_FLAG != 0 {
            PropertyKind::Template
        } else {
            PropertyKind::Literal
        }
    }
    pub fn value(&self) -> &'a [u8] {
        self.value
    }
    pub fn padding(&self) -> &'a [u8] {
        self.padding
    }

    /// A checked view, not evaluation. Malformed/binary templates stay available
    /// through `value()` and retain their unresolved template classification.
    pub fn template_ascii_cstr(&self) -> Option<&'a str> {
        if self.kind() != PropertyKind::Template {
            return None;
        }
        let text = self.value.strip_suffix(&[0])?;
        if !text.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
            return None;
        }
        core::str::from_utf8(text).ok()
    }
}

#[derive(Debug)]
pub struct FirmwareNode<'a> {
    offset: usize,
    properties: Vec<FirmwareProperty<'a>>,
    children: Vec<FirmwareNode<'a>>,
}
impl<'a> FirmwareNode<'a> {
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn properties(&self) -> &[FirmwareProperty<'a>] {
        &self.properties
    }
    pub fn children(&self) -> &[FirmwareNode<'a>] {
        &self.children
    }
}

#[derive(Debug)]
pub struct FirmwareDeviceTree<'a> {
    source: &'a [u8],
    root: FirmwareNode<'a>,
    statistics: FirmwareDtStatistics,
}
impl<'a> FirmwareDeviceTree<'a> {
    pub fn parse(source: &'a [u8]) -> Result<Self, FirmwareDtError> {
        if source.len() > MAX_INPUT_BYTES {
            return Err(FirmwareDtError::InputTooLarge);
        }
        let mut parser = Parser {
            source,
            position: 0,
            statistics: FirmwareDtStatistics::default(),
        };
        let root = parser.node(0)?;
        if parser.position != source.len() {
            return Err(FirmwareDtError::TrailingData);
        }
        Ok(Self {
            source,
            root,
            statistics: parser.statistics,
        })
    }
    pub fn source(&self) -> &'a [u8] {
        self.source
    }
    pub fn root(&self) -> &FirmwareNode<'a> {
        &self.root
    }
    pub fn statistics(&self) -> FirmwareDtStatistics {
        self.statistics
    }

    /// Only the existing runtime validator can accept unmodified runtime
    /// bytes. Any template prevents this operation, regardless of its body.
    /// Acceptance is structural and does not establish platform completeness.
    pub fn validated_runtime_bytes(&self) -> Result<&'a [u8], FirmwareDtError> {
        if self.statistics.templates != 0 {
            return Err(FirmwareDtError::UnresolvedTemplates {
                count: self.statistics.templates,
            });
        }
        crate::flat_dt::validate(self.source).map_err(FirmwareDtError::Runtime)?;
        Ok(self.source)
    }
}

struct Parser<'a> {
    source: &'a [u8],
    position: usize,
    statistics: FirmwareDtStatistics,
}
impl<'a> Parser<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], FirmwareDtError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(FirmwareDtError::Truncated)?;
        let bytes = self
            .source
            .get(self.position..end)
            .ok_or(FirmwareDtError::Truncated)?;
        self.position = end;
        Ok(bytes)
    }
    fn word(&mut self) -> Result<u32, FirmwareDtError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| FirmwareDtError::Truncated)?,
        ))
    }
    fn node(&mut self, depth: usize) -> Result<FirmwareNode<'a>, FirmwareDtError> {
        if depth > MAX_DEPTH || self.statistics.nodes == MAX_NODES {
            return Err(FirmwareDtError::LimitsExceeded);
        }
        let offset = self.position;
        let property_count = self.word()? as usize;
        let child_count = self.word()? as usize;
        if property_count > MAX_PROPERTIES_PER_NODE
            || child_count > MAX_CHILDREN_PER_NODE
            || property_count > MAX_PROPERTIES - self.statistics.properties
            || child_count > MAX_NODES - self.statistics.nodes - 1
        {
            return Err(FirmwareDtError::LimitsExceeded);
        }
        // Even an empty property occupies 36 bytes; a child needs its header.
        let minimum = property_count
            .checked_mul(36)
            .and_then(|n| n.checked_add(child_count * 8))
            .ok_or(FirmwareDtError::LimitsExceeded)?;
        if minimum > self.source.len() - self.position {
            return Err(FirmwareDtError::Truncated);
        }
        self.statistics.nodes += 1;
        self.statistics.properties += property_count;
        self.statistics.maximum_depth = self.statistics.maximum_depth.max(depth);
        let mut properties = Vec::new();
        properties
            .try_reserve_exact(property_count)
            .map_err(|_| FirmwareDtError::AllocationFailed)?;
        for _ in 0..property_count {
            let offset = self.position;
            let raw_name: &'a [u8; 32] = self
                .take(32)?
                .try_into()
                .map_err(|_| FirmwareDtError::Truncated)?;
            let terminator = raw_name.iter().position(|byte| *byte == 0).unwrap_or(32);
            if terminator == 0
                || !raw_name[..terminator]
                    .iter()
                    .all(|byte| (0x20..=0x7e).contains(byte))
                || raw_name[terminator..].iter().any(|byte| *byte != 0)
            {
                return Err(FirmwareDtError::InvalidPropertyName);
            }
            let name = core::str::from_utf8(&raw_name[..terminator])
                .map_err(|_| FirmwareDtError::InvalidPropertyName)?;
            let raw_length = self.word()?;
            let length = (raw_length & !TEMPLATE_FLAG) as usize;
            let value = self.take(length)?;
            let padding = self.take((4 - length % 4) % 4)?;
            if raw_length & TEMPLATE_FLAG != 0 {
                self.statistics.templates += 1;
            }
            properties.push(FirmwareProperty {
                offset,
                raw_name,
                name,
                raw_length,
                value,
                padding,
            });
        }
        let mut children = Vec::new();
        children
            .try_reserve_exact(child_count)
            .map_err(|_| FirmwareDtError::AllocationFailed)?;
        for _ in 0..child_count {
            children.push(self.node(depth + 1)?);
        }
        Ok(FirmwareNode {
            offset,
            properties,
            children,
        })
    }
}
