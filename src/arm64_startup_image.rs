//! Bounded physical staging of two thin ARM64e Mach-O startup companions.
//!
//! This copies public-format segment bytes into caller-owned guest RAM. Linked
//! addresses determine offsets within each physical image span and are returned
//! for separate mapping. Staging does not establish a guest mapping, relocation,
//! entry ABI, or startup readiness.

use alloc::vec::Vec;
use core::fmt;

const PAGE_SIZE: u64 = 16 * 1024;
const MAX_FILE_SIZE: usize = 128 * 1024 * 1024;
const MAX_IMAGE_SIZE: u64 = 128 * 1024 * 1024;
const MAX_COMMANDS: usize = 4096;
const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const MAX_SEGMENTS: usize = 128;
const LC_SEGMENT_64: u32 = 0x19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupImageError {
    Truncated,
    UnsupportedFormat,
    UnsupportedCpu,
    UnsupportedFileType,
    UnsupportedCommand,
    InvalidHeader,
    InvalidCommand,
    InvalidSegment,
    OverlappingRange,
    MissingSegments,
    LimitExceeded,
    AddressOverflow,
    ArenaBounds,
    AllocationFailed,
    ReadbackMismatch,
}

impl fmt::Display for StartupImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "STARTUP_IMAGE_{self:?}")
    }
}
impl core::error::Error for StartupImageError {}
type Result<T> = core::result::Result<T, StartupImageError>;
use StartupImageError as E;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagePlacement {
    /// Start of the page-rounded copy span in caller-owned physical memory.
    pub physical_base: u64,
    /// Page-aligned linked address corresponding to the physical span start.
    pub virtual_base: u64,
    /// Complete page-rounded span, including zeroed holes and segment tails.
    pub bytes: usize,
    /// Number of source bytes copied from segment file ranges.
    pub copied_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupPlacement {
    pub sptm: ImagePlacement,
    pub txm: ImagePlacement,
    /// First byte after the TXM span; an aligned value for the next allocation.
    pub occupied_end: u64,
}

#[derive(Clone, Copy)]
struct Segment {
    vmaddr: u64,
    vmsize: u64,
    fileoff: usize,
    filesize: usize,
}

struct Image<'a> {
    source: &'a [u8],
    segments: Vec<Segment>,
    virtual_base: u64,
    span: usize,
    copied_bytes: usize,
}

/// Parse and plan both companions before writing any destination byte.
///
/// `physical_base` names `memory[0]`; `occupied_end` is the exclusive end of
/// an earlier allocation in that same arena. The caller owns the source
/// authenticity check and must install any required guest mappings separately.
pub fn stage_startup_images(
    sptm: &[u8],
    txm: &[u8],
    physical_base: u64,
    occupied_end: u64,
    memory: &mut [u8],
) -> Result<StartupPlacement> {
    let sptm_image = parse_image(sptm)?;
    let txm_image = parse_image(txm)?;
    let arena_end = physical_base
        .checked_add(u64::try_from(memory.len()).map_err(|_| E::AddressOverflow)?)
        .ok_or(E::AddressOverflow)?;
    if occupied_end > arena_end {
        return Err(E::ArenaBounds);
    }
    let sptm_base = align_up(occupied_end.max(physical_base))?;
    let sptm_end = sptm_base
        .checked_add(sptm_image.span as u64)
        .ok_or(E::AddressOverflow)?;
    let txm_base = align_up(sptm_end)?;
    let txm_end = txm_base
        .checked_add(txm_image.span as u64)
        .ok_or(E::AddressOverflow)?;
    if txm_end > arena_end {
        return Err(E::ArenaBounds);
    }
    let sptm_offset = usize::try_from(sptm_base - physical_base).map_err(|_| E::ArenaBounds)?;
    let txm_offset = usize::try_from(txm_base - physical_base).map_err(|_| E::ArenaBounds)?;
    let sptm_limit = sptm_offset
        .checked_add(sptm_image.span)
        .ok_or(E::ArenaBounds)?;
    let txm_limit = txm_offset
        .checked_add(txm_image.span)
        .ok_or(E::ArenaBounds)?;
    if sptm_limit > memory.len() || txm_limit > memory.len() {
        return Err(E::ArenaBounds);
    }

    let result = StartupPlacement {
        sptm: ImagePlacement {
            physical_base: sptm_base,
            virtual_base: sptm_image.virtual_base,
            bytes: sptm_image.span,
            copied_bytes: sptm_image.copied_bytes,
        },
        txm: ImagePlacement {
            physical_base: txm_base,
            virtual_base: txm_image.virtual_base,
            bytes: txm_image.span,
            copied_bytes: txm_image.copied_bytes,
        },
        occupied_end: txm_end,
    };
    stage_image(&sptm_image, &mut memory[sptm_offset..sptm_limit])?;
    stage_image(&txm_image, &mut memory[txm_offset..txm_limit])?;
    Ok(result)
}

fn parse_image(source: &[u8]) -> Result<Image<'_>> {
    if source.len() > MAX_FILE_SIZE {
        return Err(E::LimitExceeded);
    }
    if read32(source, 0)? != 0xfeed_facf {
        return Err(E::UnsupportedFormat);
    }
    let header = bytes(source, 0, 32)?;
    if read32(header, 4)? != 0x0100_000c || read32(header, 8)? & 0x00ff_ffff != 2 {
        return Err(E::UnsupportedCpu);
    }
    if read32(header, 12)? != 2 {
        return Err(E::UnsupportedFileType);
    }
    // The selected static companion profile uses MH_NOUNDEFS and MH_PIE.
    if read32(header, 24)? & !(0x1 | 0x20_0000) != 0 {
        return Err(E::InvalidHeader);
    }
    if read32(header, 28)? != 0 {
        return Err(E::InvalidHeader);
    }
    let count = read32(header, 16)? as usize;
    let command_bytes = read32(header, 20)? as usize;
    if count > MAX_COMMANDS || command_bytes > MAX_COMMAND_BYTES {
        return Err(E::LimitExceeded);
    }
    if command_bytes % 8 != 0 || count > command_bytes / 8 {
        return Err(E::InvalidHeader);
    }
    let commands = bytes(source, 32, command_bytes)?;
    let mut segments = Vec::new();
    segments
        .try_reserve_exact(MAX_SEGMENTS)
        .map_err(|_| E::AllocationFailed)?;
    let mut offset = 0usize;
    let mut symbol_count = None;
    let mut dysymtab = None;
    let mut saw_thread = false;
    let mut pagezero_end = None;
    for _ in 0..count {
        let kind = read32(commands, offset)?;
        let size = read32(commands, offset.checked_add(4).ok_or(E::AddressOverflow)?)? as usize;
        if size < 8 || size % 8 != 0 {
            return Err(E::InvalidCommand);
        }
        let command = bytes(commands, offset, size)?;
        match kind {
            LC_SEGMENT_64 => {
                if command.len() < 72 {
                    return Err(E::InvalidCommand);
                }
                let nsects = read32(command, 64)? as usize;
                let expected = nsects
                    .checked_mul(80)
                    .and_then(|n| n.checked_add(72))
                    .ok_or(E::AddressOverflow)?;
                if expected != size || nsects > 1024 {
                    return Err(E::InvalidCommand);
                }
                let segment = Segment {
                    vmaddr: read64(command, 24)?,
                    vmsize: read64(command, 32)?,
                    fileoff: usize::try_from(read64(command, 40)?)
                        .map_err(|_| E::InvalidSegment)?,
                    filesize: usize::try_from(read64(command, 48)?)
                        .map_err(|_| E::InvalidSegment)?,
                };
                if &command[8..24] == b"__PAGEZERO\0\0\0\0\0\0" {
                    if pagezero_end.is_some()
                        || segment.vmaddr != 0
                        || segment.vmsize == 0
                        || segment.fileoff != 0
                        || segment.filesize != 0
                        || read32(command, 56)? != 0
                        || read32(command, 60)? != 0
                        || nsects != 0
                        || read32(command, 68)? != 0
                    {
                        return Err(E::InvalidSegment);
                    }
                    pagezero_end = Some(segment.vmsize);
                } else {
                    if segments.len() >= MAX_SEGMENTS
                        || segment.vmsize == 0
                        || segment.filesize as u64 > segment.vmsize
                        || segment.vmaddr.checked_add(segment.vmsize).is_none()
                    {
                        return Err(E::InvalidSegment);
                    }
                    bytes(source, segment.fileoff, segment.filesize)?;
                    for index in 0..nsects {
                        let section = &command[72 + index * 80..72 + (index + 1) * 80];
                        validate_section(source, &segment, section)?;
                    }
                    segments.push(segment);
                }
            }
            0x2 => {
                if size != 24 || symbol_count.is_some() {
                    return Err(E::InvalidCommand);
                }
                let nsyms = read32(command, 12)?;
                file_table(source, read32(command, 8)?, nsyms, 16)?;
                file_table(source, read32(command, 16)?, read32(command, 20)?, 1)?;
                symbol_count = Some(nsyms);
            }
            0xb => {
                if size != 80 || dysymtab.is_some() {
                    return Err(E::InvalidCommand);
                }
                for (at, width) in [(32, 8), (40, 56), (48, 4), (56, 4), (64, 8), (72, 8)] {
                    file_table(
                        source,
                        read32(command, at)?,
                        read32(command, at + 4)?,
                        width,
                    )?;
                }
                dysymtab = Some([
                    (read32(command, 8)?, read32(command, 12)?),
                    (read32(command, 16)?, read32(command, 20)?),
                    (read32(command, 24)?, read32(command, 28)?),
                ]);
            }
            0x1b if size == 24 => {}
            0x2a if size == 16 => {}
            0x5 => {
                if saw_thread || size < 16 {
                    return Err(E::InvalidCommand);
                }
                let words = read32(command, 12)? as usize;
                if words != 68
                    || words.checked_mul(4).and_then(|n| n.checked_add(16)) != Some(size)
                    || read32(command, 8)? != 6
                {
                    return Err(E::InvalidCommand);
                }
                saw_thread = true;
            }
            0x1b | 0x2a => return Err(E::InvalidCommand),
            _ => return Err(E::UnsupportedCommand),
        }
        offset = offset.checked_add(size).ok_or(E::AddressOverflow)?;
    }
    if offset != commands.len() {
        return Err(E::InvalidCommand);
    }
    if let Some(ranges) = dysymtab {
        let total = symbol_count.ok_or(E::InvalidCommand)?;
        for (first, count) in ranges {
            if first.checked_add(count).ok_or(E::AddressOverflow)? > total {
                return Err(E::InvalidCommand);
            }
        }
    }
    if segments.is_empty() {
        return Err(E::MissingSegments);
    }
    segments.sort_unstable_by_key(|s| s.vmaddr);
    if pagezero_end.is_some_and(|end| end > segments[0].vmaddr) {
        return Err(E::OverlappingRange);
    }
    let header_bytes = 32 + command_bytes;
    if !segments
        .iter()
        .any(|s| s.fileoff == 0 && s.filesize >= header_bytes)
    {
        return Err(E::InvalidHeader);
    }
    for pair in segments.windows(2) {
        if pair[0].vmaddr + pair[0].vmsize > pair[1].vmaddr {
            return Err(E::OverlappingRange);
        }
    }
    for (index, segment) in segments.iter().enumerate() {
        for other in &segments[..index] {
            if overlaps(
                segment.fileoff as u64,
                segment.filesize as u64,
                other.fileoff as u64,
                other.filesize as u64,
            ) {
                return Err(E::OverlappingRange);
            }
        }
    }
    let virtual_base = segments[0].vmaddr & !(PAGE_SIZE - 1);
    let last = segments.last().ok_or(E::MissingSegments)?;
    let virtual_end = align_up(last.vmaddr + last.vmsize)?;
    let span = virtual_end
        .checked_sub(virtual_base)
        .ok_or(E::AddressOverflow)?;
    if span == 0 || span > MAX_IMAGE_SIZE || span > isize::MAX as u64 {
        return Err(E::LimitExceeded);
    }
    let copied_bytes = segments.iter().try_fold(0usize, |sum, s| {
        sum.checked_add(s.filesize).ok_or(E::AddressOverflow)
    })?;
    Ok(Image {
        source,
        segments,
        virtual_base,
        span: span as usize,
        copied_bytes,
    })
}

fn validate_section(source: &[u8], segment: &Segment, section: &[u8]) -> Result<()> {
    let address = read64(section, 32)?;
    let size = read64(section, 40)?;
    let end = address.checked_add(size).ok_or(E::AddressOverflow)?;
    let segment_end = segment.vmaddr + segment.vmsize;
    if address < segment.vmaddr || end > segment_end || read32(section, 52)? > 31 {
        return Err(E::InvalidSegment);
    }
    let section_type = read32(section, 64)? & 0xff;
    if !matches!(section_type, 0x1 | 0xc | 0x12) && size != 0 {
        let fileoff = read32(section, 48)? as u64;
        let file_end = fileoff.checked_add(size).ok_or(E::AddressOverflow)?;
        let segment_file_end = (segment.fileoff + segment.filesize) as u64;
        if fileoff < segment.fileoff as u64 || file_end > segment_file_end {
            return Err(E::InvalidSegment);
        }
    }
    file_table(source, read32(section, 56)?, read32(section, 60)?, 8)?;
    Ok(())
}

fn stage_image(image: &Image<'_>, destination: &mut [u8]) -> Result<()> {
    destination.fill(0);
    for segment in &image.segments {
        let offset = (segment.vmaddr - image.virtual_base) as usize;
        let source = &image.source[segment.fileoff..segment.fileoff + segment.filesize];
        destination[offset..offset + segment.filesize].copy_from_slice(source);
    }
    let mut checked_end = 0;
    for segment in &image.segments {
        let start = (segment.vmaddr - image.virtual_base) as usize;
        let file_end = start + segment.filesize;
        let memory_end = start + segment.vmsize as usize;
        verify_zero(destination, checked_end, start)?;
        verify_bytes(
            destination,
            start,
            &image.source[segment.fileoff..segment.fileoff + segment.filesize],
        )?;
        verify_zero(destination, file_end, memory_end)?;
        checked_end = memory_end;
    }
    verify_zero(destination, checked_end, destination.len())?;
    Ok(())
}

fn verify_zero(destination: &[u8], start: usize, end: usize) -> Result<()> {
    let region = destination.get(start..end).ok_or(E::ArenaBounds)?;
    for index in 0..region.len() {
        // SAFETY: index is bounded by the validated region slice.
        if unsafe { core::ptr::read_volatile(region.as_ptr().add(index)) } != 0 {
            return Err(E::ReadbackMismatch);
        }
    }
    Ok(())
}

fn verify_bytes(destination: &[u8], start: usize, expected: &[u8]) -> Result<()> {
    let end = start.checked_add(expected.len()).ok_or(E::ArenaBounds)?;
    let region = destination.get(start..end).ok_or(E::ArenaBounds)?;
    for (index, &byte) in expected.iter().enumerate() {
        // SAFETY: expected and region have equal length, so index is in bounds.
        if unsafe { core::ptr::read_volatile(region.as_ptr().add(index)) } != byte {
            return Err(E::ReadbackMismatch);
        }
    }
    Ok(())
}

fn file_table(source: &[u8], offset: u32, count: u32, width: usize) -> Result<()> {
    let size = (count as usize)
        .checked_mul(width)
        .ok_or(E::AddressOverflow)?;
    bytes(source, offset as usize, size)?;
    Ok(())
}

fn bytes(source: &[u8], offset: usize, size: usize) -> Result<&[u8]> {
    let end = offset.checked_add(size).ok_or(E::AddressOverflow)?;
    source.get(offset..end).ok_or(E::Truncated)
}

fn read32(source: &[u8], offset: usize) -> Result<u32> {
    let word: [u8; 4] = bytes(source, offset, 4)?
        .try_into()
        .map_err(|_| E::Truncated)?;
    Ok(u32::from_le_bytes(word))
}

fn read64(source: &[u8], offset: usize) -> Result<u64> {
    let word: [u8; 8] = bytes(source, offset, 8)?
        .try_into()
        .map_err(|_| E::Truncated)?;
    Ok(u64::from_le_bytes(word))
}

fn align_up(value: u64) -> Result<u64> {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|n| n & !(PAGE_SIZE - 1))
        .ok_or(E::AddressOverflow)
}

fn overlaps(a: u64, a_len: u64, b: u64, b_len: u64) -> bool {
    a_len != 0 && b_len != 0 && a < b + b_len && b < a + a_len
}
