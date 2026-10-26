//! The canonical ZIP layout of an MCS archive, read byte by byte.
//!
//! MCS admits exactly one layout: local members in central-directory order
//! from offset 0 with no gap, no extra fields, no comments, no data
//! descriptors, no ZIP64, a fixed timestamp and fixed attributes, and an end
//! record with nothing after it. Anything else is refused before a member is
//! decompressed, so no reader can see a member another reader would not.

use std::collections::BTreeSet;
use std::io::Read as _;

use unicode_normalization::is_nfc;

use crate::{
    MAX_COMPRESSED, MAX_ENTRIES, MAX_FILE, MAX_METADATA, MAX_RATIO, MAX_TOTAL, METADATA, McsError,
    NAME_MAX,
};

const LOCAL: &[u8; 4] = b"PK\x03\x04";
const CENTRAL: &[u8; 4] = b"PK\x01\x02";
const END: &[u8; 4] = b"PK\x05\x06";
const END_LEN: usize = 22;
const LOCAL_LEN: usize = 30;
const CENTRAL_LEN: usize = 46;
/// ZIP 2.0, made on Unix.
const MADE_BY: u16 = (3 << 8) | 0x14;
const NEEDED: u16 = 20;
/// 1980-01-01 00:00:00 in MS-DOS time and date.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 33;
/// A regular file, mode 0644.
const EXTERNAL: u32 = (0o100_644) << 16;
const UTF8_NAME: u16 = 0x800;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

/// One member, in archive order.
pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
}

struct Entry {
    name: String,
    flags: u16,
    compression: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
}

fn container(detail: &str) -> McsError {
    McsError::Container(detail.to_owned())
}

fn u16_at(raw: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([raw[at], raw[at + 1]])
}

fn u32_at(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Refuses a member name MCS does not admit: empty, too long, a backslash,
/// a control character, not NFC, absolute, or with an empty, `.` or `..`
/// segment.
pub(crate) fn safe_name(name: &str) -> Result<(), McsError> {
    let unsafe_name = || McsError::UnsafeName(name.to_owned());
    if name.is_empty()
        || name.chars().count() > NAME_MAX
        || name.contains('\\')
        || name.chars().any(|c| u32::from(c) < 32)
        || !is_nfc(name)
        || name.starts_with('/')
        || name
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(unsafe_name());
    }
    Ok(())
}

/// The key two names collide under: NFC, then full Unicode default case
/// folding, the MCS reader's `unicodedata.normalize("NFC", name).casefold()`.
///
/// Folding is not upper- then lowercasing: `ẞ` folds to `ss` but lowercases
/// to `ß`, and a final sigma folds to `σ`.
fn folded(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    caseless::default_case_fold_str(&name.nfc().collect::<String>())
}

fn entry_name(raw: &[u8], flags: u16) -> Result<String, McsError> {
    // A name is UTF-8 flagged exactly when it is not ASCII.
    if flags == 0 {
        if !raw.is_ascii() {
            return Err(container(
                "non-canonical or unsupported ZIP member metadata",
            ));
        }
    } else if flags != UTF8_NAME {
        return Err(container(
            "non-canonical or unsupported ZIP member metadata",
        ));
    }
    let name = String::from_utf8(raw.to_vec())
        .map_err(|_| container("non-canonical or unsupported ZIP member metadata"))?;
    if flags == UTF8_NAME && name.is_ascii() {
        return Err(container(
            "non-canonical or unsupported ZIP member metadata",
        ));
    }
    Ok(name)
}

/// The central directory, checked against the end record.
fn central_directory(raw: &[u8]) -> Result<(Vec<Entry>, usize), McsError> {
    if raw.len() < END_LEN {
        return Err(container("truncated ZIP archive"));
    }
    let end = raw.len() - END_LEN;
    let count = usize::from(u16_at(raw, end + 10));
    let size = to_usize(u32_at(raw, end + 12));
    let offset = to_usize(u32_at(raw, end + 16));
    if &raw[end..end + 4] != END
        || u16_at(raw, end + 4) != 0
        || u16_at(raw, end + 6) != 0
        || usize::from(u16_at(raw, end + 8)) != count
        || u16_at(raw, end + 20) != 0
        || offset.checked_add(size) != Some(end)
    {
        return Err(container("non-canonical ZIP end record or trailing data"));
    }
    if !(3..=MAX_ENTRIES).contains(&count) {
        return Err(container("invalid archive entry count"));
    }
    let mut entries = Vec::with_capacity(count);
    let mut cursor = offset;
    for _ in 0..count {
        if cursor + CENTRAL_LEN > end {
            return Err(container("ZIP central directory contains undeclared data"));
        }
        let at = cursor;
        let name_length = usize::from(u16_at(raw, at + 28));
        let name_start = at + CENTRAL_LEN;
        if name_start + name_length > end {
            return Err(container("ZIP central directory contains undeclared data"));
        }
        let flags = u16_at(raw, at + 8);
        if &raw[at..at + 4] != CENTRAL
            || u16_at(raw, at + 4) != MADE_BY
            || u16_at(raw, at + 6) != NEEDED
            || u16_at(raw, at + 12) != DOS_TIME
            || u16_at(raw, at + 14) != DOS_DATE
            || u16_at(raw, at + 30) != 0
            || u16_at(raw, at + 32) != 0
            || u16_at(raw, at + 34) != 0
            || u16_at(raw, at + 36) != 0
            || u32_at(raw, at + 38) != EXTERNAL
        {
            return Err(container("non-canonical ZIP central header"));
        }
        entries.push(Entry {
            name: entry_name(&raw[name_start..name_start + name_length], flags)?,
            flags,
            compression: u16_at(raw, at + 10),
            crc: u32_at(raw, at + 16),
            compressed: u32_at(raw, at + 20),
            size: u32_at(raw, at + 24),
            offset: u32_at(raw, at + 42),
        });
        cursor = name_start + name_length;
    }
    if cursor != end {
        return Err(container("ZIP central directory contains undeclared data"));
    }
    Ok((entries, offset))
}

/// Every member's checks before any is read: its name, its metadata, the
/// limits, and the place of `mimetype` and the metadata.
fn check_entries(entries: &[Entry]) -> Result<(), McsError> {
    let mut names = BTreeSet::new();
    let mut folds = BTreeSet::new();
    let (mut total, mut compressed) = (0_u64, 0_u64);
    for (index, entry) in entries.iter().enumerate() {
        safe_name(&entry.name)?;
        if !names.insert(entry.name.as_str()) || !folds.insert(folded(&entry.name)) {
            return Err(McsError::DuplicateMember(entry.name.clone()));
        }
        let expected = if index == 0 { STORED } else { DEFLATED };
        if entry.compression != expected {
            return Err(container(
                "non-canonical or unsupported ZIP member metadata",
            ));
        }
        if to_usize(entry.size) > MAX_FILE || to_usize(entry.compressed) > MAX_FILE {
            return Err(McsError::Limit(format!(
                "member {:?} exceeds {MAX_FILE} bytes",
                entry.name
            )));
        }
        if entry.compressed != 0
            && u64::from(entry.size) > u64::from(MAX_RATIO) * u64::from(entry.compressed)
        {
            return Err(McsError::Limit(format!(
                "member {:?} is compressed more than {MAX_RATIO} to 1",
                entry.name
            )));
        }
        total += u64::from(entry.size);
        compressed += u64::from(entry.compressed);
        if total > MAX_TOTAL as u64 || compressed > MAX_COMPRESSED as u64 {
            return Err(McsError::Limit(
                "the archive's members exceed the total limit".into(),
            ));
        }
        if index == 0 && entry.name != "mimetype" {
            return Err(container("mimetype must be first and stored"));
        }
        if index == 1 && (entry.name != METADATA || to_usize(entry.size) > MAX_METADATA) {
            return Err(container("metadata must be second and bounded"));
        }
    }
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// The members of a canonical MCS container, in archive order.
pub(crate) fn members(raw: &[u8]) -> Result<Vec<Member>, McsError> {
    if raw.len() > MAX_COMPRESSED {
        return Err(McsError::Limit(format!(
            "the archive is larger than {MAX_COMPRESSED} bytes"
        )));
    }
    let (entries, central) = central_directory(raw)?;
    check_entries(&entries)?;
    let mut cursor = 0_usize;
    let mut members = Vec::with_capacity(entries.len());
    for entry in &entries {
        if to_usize(entry.offset) != cursor || cursor + LOCAL_LEN > central {
            return Err(container(
                "ZIP contains a preamble, gap, or overlapping local member",
            ));
        }
        let at = cursor;
        let name = entry.name.as_bytes();
        let name_length = usize::from(u16_at(raw, at + 26));
        let data_start = at + LOCAL_LEN + name_length;
        let data_end = data_start + to_usize(entry.compressed);
        if &raw[at..at + 4] != LOCAL
            || u16_at(raw, at + 4) != NEEDED
            || u16_at(raw, at + 6) != entry.flags
            || u16_at(raw, at + 8) != entry.compression
            || u16_at(raw, at + 10) != DOS_TIME
            || u16_at(raw, at + 12) != DOS_DATE
            || u32_at(raw, at + 14) != entry.crc
            || u32_at(raw, at + 18) != entry.compressed
            || u32_at(raw, at + 22) != entry.size
            || u16_at(raw, at + 28) != 0
            || name_length != name.len()
            || data_end > central
            || &raw[at + LOCAL_LEN..data_start] != name
        {
            return Err(container("non-canonical ZIP local header"));
        }
        let stored = &raw[data_start..data_end];
        let data = if entry.compression == STORED {
            stored.to_vec()
        } else {
            let mut data = Vec::new();
            flate2::read::DeflateDecoder::new(stored)
                .take(u64::from(entry.size) + 1)
                .read_to_end(&mut data)
                .map_err(|error| {
                    container(&format!(
                        "member {:?} cannot be inflated: {error}",
                        entry.name
                    ))
                })?;
            data
        };
        if data.len() != to_usize(entry.size) || crc32(&data) != entry.crc {
            return Err(container(&format!(
                "member {:?} does not match its size or CRC-32",
                entry.name
            )));
        }
        members.push(Member {
            name: entry.name.clone(),
            data,
        });
        cursor = data_end;
    }
    if cursor != central {
        return Err(container(
            "ZIP contains data outside declared local members",
        ));
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::folded;

    /// Each name with its key as the MCS reader folds it, cross-checked with
    /// `python3 -c "import unicodedata as u; print(u.normalize('NFC', '…').casefold())"`
    /// (Python 3.13, Unicode 15.1).
    #[test]
    fn names_fold_as_python_casefolds_them() {
        for (name, key) in [
            ("source/STRASSE.txt", "source/strasse.txt"),
            ("source/straße.txt", "source/strasse.txt"),
            ("source/STRAẞE.txt", "source/strasse.txt"),
            ("source/ẞ", "source/ss"),
            ("source/ΟΔΟΣ.txt", "source/οδοσ.txt"),
            ("source/οδος.txt", "source/οδοσ.txt"),
            ("source/Ärger.txt", "source/ärger.txt"),
            ("source/ärger.txt", "source/ärger.txt"),
            ("source/A\u{308}rger.txt", "source/ärger.txt"),
            ("source/ﬁle", "source/file"),
            ("source/İ", "source/i\u{307}"),
        ] {
            assert_eq!(folded(name), key, "{name:?}");
        }
    }
}
