//! Unpacks a fixture archive and packs it again canonically, so a test can
//! change one thing and see the reader refuse exactly that.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../fixtures/mcs-v0.1.0");

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{name}")).unwrap()
}

/// An archive's metadata and payload members (all but `mimetype` and the
/// metadata).
pub struct Parts {
    pub metadata: Value,
    pub files: BTreeMap<String, Vec<u8>>,
}

fn u16_at(raw: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([raw[at], raw[at + 1]]))
}

fn u32_at(raw: &[u8], at: usize) -> usize {
    u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]) as usize
}

/// The members of a canonical archive, read through its local headers.
pub fn members(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut members = Vec::new();
    let mut at = 0;
    while raw[at..].starts_with(b"PK\x03\x04") {
        let compression = u16_at(raw, at + 8);
        let compressed = u32_at(raw, at + 18);
        let name_length = u16_at(raw, at + 26);
        let start = at + 30 + name_length;
        let name = String::from_utf8(raw[at + 30..start].to_vec()).unwrap();
        let stored = &raw[start..start + compressed];
        let data = if compression == 0 {
            stored.to_vec()
        } else {
            let mut data = Vec::new();
            flate2::read::DeflateDecoder::new(stored)
                .read_to_end(&mut data)
                .unwrap();
            data
        };
        members.push((name, data));
        at = start + compressed;
    }
    members
}

pub fn unpack(raw: &[u8]) -> Parts {
    let mut files: BTreeMap<String, Vec<u8>> = members(raw).into_iter().collect();
    files.remove("mimetype");
    let metadata = serde_json::from_slice(&files.remove("META-INF/mcs.json").unwrap()).unwrap();
    Parts { metadata, files }
}

fn role(name: &str) -> &'static str {
    if name.starts_with("source/") {
        "source"
    } else if name == "normalized/ruleset.json" {
        "ruleset"
    } else {
        "definition"
    }
}

impl Parts {
    pub fn json(&self, name: &str) -> Value {
        serde_json::from_slice(&self.files[name]).unwrap()
    }

    pub fn set_json(&mut self, name: &str, value: &Value) {
        self.files
            .insert(name.to_owned(), serde_json::to_vec(value).unwrap());
    }

    /// The archived manifest's member name.
    pub fn manifest(&self) -> String {
        self.metadata["manifest"].as_str().unwrap().to_owned()
    }

    /// The normalized member a source module maps to.
    pub fn normalized(&self, module: &str) -> String {
        self.metadata["normalized"][module]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Recomputes the inventory and packs the members canonically.
    pub fn pack(&mut self) -> Vec<u8> {
        self.metadata["inventory"] = Value::Array(
            self.files
                .iter()
                .map(|(name, data)| {
                    serde_json::json!({
                        "path": name,
                        "role": role(name),
                        "size": data.len(),
                        "sha256": hex(&Sha256::digest(data)),
                    })
                })
                .collect(),
        );
        let mut members = vec![
            (
                "mimetype".to_owned(),
                b"application/vnd.axioval.mcs+zip".to_vec(),
            ),
            (
                "META-INF/mcs.json".to_owned(),
                serde_json::to_vec(&self.metadata).unwrap(),
            ),
        ];
        members.extend(self.files.iter().map(|(n, d)| (n.clone(), d.clone())));
        write_zip(&members)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn crc(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// A canonical MCS container of `members`, in that order: the first stored,
/// the rest deflated.
pub fn write_zip(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (index, (name, data)) in members.iter().enumerate() {
        let (method, stored) = if index == 0 {
            (0_u16, data.clone())
        } else {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(data).unwrap();
            (8, encoder.finish().unwrap())
        };
        let flags: u16 = if name.is_ascii() { 0 } else { 0x800 };
        let offset = u32::try_from(out.len()).unwrap();
        let fields = |header: &mut Vec<u8>| {
            header.extend(flags.to_le_bytes());
            header.extend(method.to_le_bytes());
            header.extend(0_u16.to_le_bytes());
            header.extend(33_u16.to_le_bytes());
            header.extend(crc(data).to_le_bytes());
            header.extend(u32::try_from(stored.len()).unwrap().to_le_bytes());
            header.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
            header.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
            header.extend(0_u16.to_le_bytes());
        };
        out.extend(b"PK\x03\x04");
        out.extend(20_u16.to_le_bytes());
        fields(&mut out);
        out.extend(name.as_bytes());
        out.extend(&stored);
        central.extend(b"PK\x01\x02");
        central.extend(((3_u16 << 8) | 20).to_le_bytes());
        central.extend(20_u16.to_le_bytes());
        fields(&mut central);
        central.extend(0_u16.to_le_bytes()); // comment
        central.extend(0_u16.to_le_bytes()); // disk
        central.extend(0_u16.to_le_bytes()); // internal attributes
        central.extend(((0o100_644_u32) << 16).to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let offset = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(members.len()).unwrap();
    let size = u32::try_from(central.len()).unwrap();
    out.extend(central);
    out.extend(b"PK\x05\x06");
    out.extend(0_u16.to_le_bytes());
    out.extend(0_u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(offset.to_le_bytes());
    out.extend(0_u16.to_le_bytes());
    out
}
