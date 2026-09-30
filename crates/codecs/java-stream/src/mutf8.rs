//! Modified UTF-8, the string encoding of the stream protocol.
//!
//! Java encodes a string as its UTF-16 code units, each written as one, two
//! or three bytes: `U+0000` takes two bytes (`C0 80`), and a supplementary
//! character is a surrogate pair of two three-byte units. A decoder accepts
//! any byte sequence of those shapes, including non-shortest forms, so the
//! bytes are kept as read and decoded on demand.

use std::fmt;

/// A modified UTF-8 byte string, kept exactly as it was read.
///
/// The bytes are validated to have the shapes a Java decoder accepts; the
/// value may still hold a lone surrogate or a non-shortest form, which
/// [`Mutf8::to_utf16`] decodes and [`Mutf8::to_string_lossless`] refuses.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Mutf8(Vec<u8>);

/// Bytes that are no modified UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid modified UTF-8 at byte {index}")]
pub struct Mutf8Error {
    /// Offset of the first byte of the malformed unit.
    pub index: usize,
}

/// A decoded string holding a lone surrogate, which Rust strings cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("modified UTF-8 holds a lone surrogate {unit:#06x}")]
pub struct LoneSurrogate {
    /// The unpaired UTF-16 code unit.
    pub unit: u16,
}

impl Mutf8 {
    /// Wraps bytes after checking they are modified UTF-8.
    ///
    /// # Errors
    ///
    /// [`Mutf8Error`] when a unit has no valid lead byte or continuation.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Mutf8Error> {
        validate(&bytes)?;
        Ok(Self(bytes))
    }

    /// Wraps bytes [`validate`] accepted.
    pub(crate) fn from_validated(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Encodes a Rust string the way `DataOutput.writeUTF` does.
    pub fn encode(text: &str) -> Self {
        let mut bytes = Vec::with_capacity(text.len());
        for unit in text.encode_utf16() {
            push_unit(&mut bytes, unit);
        }
        Self(bytes)
    }

    /// Encodes UTF-16 code units, lone surrogates included.
    pub fn from_utf16(units: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(units.len());
        for &unit in units {
            push_unit(&mut bytes, unit);
        }
        Self(bytes)
    }

    /// The encoded bytes, exactly as read or built.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The encoded length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consumes the value, returning its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Decodes the UTF-16 code units, as `DataInput.readUTF` does.
    pub fn to_utf16(&self) -> Vec<u16> {
        let bytes = &self.0;
        let mut units = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            let lead = bytes[index];
            let (unit, width) = match lead >> 4 {
                0..=7 => (u16::from(lead), 1),
                12 | 13 => (
                    (u16::from(lead & 0x1F) << 6) | u16::from(bytes[index + 1] & 0x3F),
                    2,
                ),
                _ => (
                    (u16::from(lead & 0x0F) << 12)
                        | (u16::from(bytes[index + 1] & 0x3F) << 6)
                        | u16::from(bytes[index + 2] & 0x3F),
                    3,
                ),
            };
            units.push(unit);
            index += width;
        }
        units
    }

    /// Decodes to a Rust string.
    ///
    /// # Errors
    ///
    /// [`LoneSurrogate`] when the Java string holds an unpaired surrogate.
    pub fn to_string_lossless(&self) -> Result<String, LoneSurrogate> {
        char::decode_utf16(self.to_utf16())
            .map(|unit| {
                unit.map_err(|error| LoneSurrogate {
                    unit: error.unpaired_surrogate(),
                })
            })
            .collect()
    }

    /// Decodes to a Rust string, replacing lone surrogates with `U+FFFD`.
    pub fn to_string_lossy(&self) -> String {
        char::decode_utf16(self.to_utf16())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }
}

impl From<&str> for Mutf8 {
    fn from(text: &str) -> Self {
        Self::encode(text)
    }
}

impl PartialEq<str> for Mutf8 {
    fn eq(&self, other: &str) -> bool {
        // Only the canonical encoding compares equal, as bytes on the wire.
        let mut encoded = Vec::with_capacity(other.len());
        for unit in other.encode_utf16() {
            push_unit(&mut encoded, unit);
        }
        self.0 == encoded
    }
}

impl PartialEq<&str> for Mutf8 {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl fmt::Debug for Mutf8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_string_lossy(), f)
    }
}

impl fmt::Display for Mutf8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

fn push_unit(bytes: &mut Vec<u8>, unit: u16) {
    // Truncating casts are the encoding: each keeps the low bits it shifts in.
    #[allow(clippy::cast_possible_truncation)]
    match unit {
        0x0001..=0x007F => bytes.push(unit as u8),
        0x0000 | 0x0080..=0x07FF => {
            bytes.push(0xC0 | (unit >> 6) as u8);
            bytes.push(0x80 | (unit & 0x3F) as u8);
        }
        _ => {
            bytes.push(0xE0 | (unit >> 12) as u8);
            bytes.push(0x80 | ((unit >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (unit & 0x3F) as u8);
        }
    }
}

/// Checks the unit shapes a Java decoder accepts.
pub(crate) fn validate(bytes: &[u8]) -> Result<(), Mutf8Error> {
    let continuation = |index: usize| bytes.get(index).is_some_and(|b| b & 0xC0 == 0x80);
    let mut index = 0;
    while index < bytes.len() {
        let width = match bytes[index] >> 4 {
            0..=7 => 1,
            12 | 13 if continuation(index + 1) => 2,
            14 if continuation(index + 1) && continuation(index + 2) => 3,
            _ => return Err(Mutf8Error { index }),
        };
        index += width;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_nul_and_supplementary_characters_as_java_does() {
        assert_eq!(Mutf8::encode("a\0b").as_bytes(), b"a\xC0\x80b");
        let astral = Mutf8::encode("\u{1F600}");
        assert_eq!(astral.as_bytes(), [0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
        assert_eq!(astral.to_string_lossless().unwrap(), "\u{1F600}");
        assert_eq!(
            Mutf8::encode("é€").as_bytes(),
            [0xC3, 0xA9, 0xE2, 0x82, 0xAC]
        );
    }

    #[test]
    fn keeps_non_shortest_forms_and_lone_surrogates() {
        let overlong = Mutf8::from_bytes(vec![0xC1, 0x81]).unwrap();
        assert_eq!(overlong.to_utf16(), [0x41]);
        assert_ne!(overlong, "A");
        let lone = Mutf8::from_utf16(&[0x78, 0xD800]);
        assert_eq!(
            lone.to_string_lossless(),
            Err(LoneSurrogate { unit: 0xD800 })
        );
        assert_eq!(lone.to_string_lossy(), "x\u{FFFD}");
    }

    #[test]
    fn refuses_malformed_units() {
        for bad in [
            &[0x80][..],
            &[0xC3],
            &[0xE2, 0x82],
            &[0xF0, 0x9F, 0x98, 0x80],
            &[0xC3, 0x41],
        ] {
            assert!(Mutf8::from_bytes(bad.to_vec()).is_err(), "{bad:x?}");
        }
    }
}
