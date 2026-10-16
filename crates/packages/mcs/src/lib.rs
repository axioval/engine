//! Axioval MCS package archives (`.mcs`), read fail closed.
//!
//! An MCS archive is the transport of an [Axioval MCS] package: a
//! deterministic ZIP container holding the reviewed Pkl sources under
//! `source/`, the normalized JSON the engine binds under `normalized/`, and
//! `META-INF/mcs.json`, which inventories every member by size and SHA-256.
//! [`read`] checks the container exactly as the MCS reader
//! (`scripts/mcs_archive.py`) does and returns the normalized packages as
//! the IR's [`RuleSetPackage`] and [`DefinitionPackage`], with the table
//! files their parameters name loaded from the archive.
//!
//! The Pkl sources are never evaluated: the engine binds the normalized
//! JSON, as it binds a package written as JSON, and runs no package code.
//!
//! # Two kinds
//!
//! The archived manifest (`axioval.json`) states the kind:
//!
//! - **`ruleset`**: one normalized rule set (`normalized/ruleset.json`) and
//!   the definition documents it carries (`normalized/definitions/*.json`).
//! - **`definitions`**: a library of definitions, such as rule templates,
//!   with no rule set. Its manifest's identity is the package identity of
//!   exactly one of its definition documents.
//!
//! A rule set names the definition packages it binds by ID
//! (`definitionPackages`). [`resolve_definitions`] joins the ones a host
//! supplies (definitions archives, or JSON) with the ones rule-set archives
//! carry; the compiler refuses an ID neither gives.
//!
//! # Fail closed
//!
//! Refused, each as its own [`McsError`]: a non-canonical container
//! (preamble, gap, trailing data, extra fields, comments, a timestamp or
//! attributes other than the fixed ones), an unsafe member name (absolute,
//! `..`, backslash, control character, not NFC), a duplicate or case-folded
//! duplicate member, a member over 10 MB, compressed more than 100 to 1, a
//! total over 64 MiB, an inventory that disagrees with the members, metadata
//! whose package identity differs from the archived manifest's, a
//! definitions archive carrying a rule set, and a rule-set archive without
//! one.
//!
//! [Axioval MCS]: https://github.com/axioval/mcs

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use axioval_engine::{PackageFiles, load_definition_table_files, load_table_files};
use axioval_ir::contract::{DefinitionPackage, RuleSetPackage};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod container;
mod json;

/// The media type, stored uncompressed as the first member, `mimetype`.
pub const MEDIA_TYPE: &str = "application/vnd.axioval.mcs+zip";
/// The metadata's `format`.
pub const FORMAT: &str = "axioval.mcs";
/// The container format version this reader reads.
pub const FORMAT_VERSION: &str = "0.1.0";
/// The metadata member, second in the archive.
pub const METADATA: &str = "META-INF/mcs.json";
/// The normalized rule set of a `ruleset` package.
pub const RULESET_MEMBER: &str = "normalized/ruleset.json";
/// The most members an archive holds.
pub const MAX_ENTRIES: usize = 512;
/// The largest member, compressed or not, in bytes.
pub const MAX_FILE: usize = 10_000_000;
/// The largest total of the members' sizes, in bytes.
pub const MAX_TOTAL: usize = 64 * 1024 * 1024;
/// The largest archive, and total of compressed sizes, in bytes.
pub const MAX_COMPRESSED: usize = 64 * 1024 * 1024;
/// The highest ratio of a member's size to its compressed size.
pub const MAX_RATIO: u32 = 100;
/// The largest metadata member, in bytes.
pub const MAX_METADATA: usize = 256 * 1024;
/// The longest member name, in characters.
pub const NAME_MAX: usize = 240;

const MANIFEST_VERSION: &str = "0.1.0";
const SCHEMA_VERSION: &str = "0.1.0";

/// What a package is, as its archived manifest states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PackageKind {
    /// A rule set with the definition documents it carries.
    RuleSet,
    /// A library of definitions without a rule set.
    Definitions,
}

impl fmt::Display for PackageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RuleSet => "ruleset",
            Self::Definitions => "definitions",
        })
    }
}

/// A package's identity, as the metadata and the manifest state it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageIdentity {
    /// The qualified package ID.
    pub id: String,
    /// The package's semantic version.
    pub version: String,
    /// The MCS schema version the package is written in.
    pub schema_version: String,
}

impl fmt::Display for PackageIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}@{} (schema {})",
            self.id, self.version, self.schema_version
        )
    }
}

/// Why an archive, or the packages read from several, were refused.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum McsError {
    /// The file could not be read.
    #[error("{0}")]
    Read(String),
    /// The ZIP container is not the canonical MCS layout.
    #[error("not a canonical MCS container: {0}")]
    Container(String),
    /// A size, ratio or total limit is exceeded.
    #[error("archive limit exceeded: {0}")]
    Limit(String),
    /// A member name could escape the archive or is not canonical.
    #[error("unsafe member name {0:?}")]
    UnsafeName(String),
    /// Two members share a name, or names equal once case folded.
    #[error("duplicate or casefold-colliding member {0:?}")]
    DuplicateMember(String),
    /// The metadata, its inventory or its normalized mapping is invalid.
    #[error("invalid metadata: {0}")]
    Metadata(String),
    /// The archived manifest is invalid.
    #[error("invalid archived manifest: {0}")]
    Manifest(String),
    /// The metadata's package identity differs from the archived manifest's.
    #[error(
        "metadata package identity {metadata} disagrees with the archived manifest's {manifest}"
    )]
    IdentityMismatch {
        /// The identity `META-INF/mcs.json` states.
        metadata: Box<PackageIdentity>,
        /// The identity the archived `axioval.json` states.
        manifest: Box<PackageIdentity>,
    },
    /// A `definitions` archive carries a normalized rule set.
    #[error("a definitions package must not carry a normalized rule set")]
    DefinitionsCarryRuleSet,
    /// A `ruleset` archive carries no normalized rule set.
    #[error("a ruleset package must carry exactly one normalized rule set")]
    RuleSetMissing,
    /// A `definitions` manifest's identity is the package identity of no
    /// definition document, or of several.
    #[error(
        "manifest identity {id}@{version} is not the package identity of one definition \
         document ({matches} match)"
    )]
    DefinitionsIdentity {
        /// The manifest's package ID.
        id: String,
        /// The manifest's package version.
        version: String,
        /// How many definition documents have that identity.
        matches: usize,
    },
    /// A normalized package disagrees with the archive, or does not read as
    /// the package contract.
    #[error("{member}: {detail}")]
    Package {
        /// The normalized member.
        member: String,
        /// What is wrong with it.
        detail: String,
    },
    /// The definition documents do not bind together.
    #[error("definition documents do not bind: {0}")]
    Binding(String),
    /// A table file a parameter names was refused.
    #[error("{0}")]
    TableFile(String),
    /// The archive is of the other kind than the one asked for.
    #[error("a {found} archive was given where a {expected} archive is expected")]
    WrongKind {
        /// The kind asked for.
        expected: PackageKind,
        /// The kind the archive's manifest states.
        found: PackageKind,
    },
    /// Two definition packages with one ID differ.
    #[error("two different definition packages are named {0:?}")]
    ConflictingDefinitions(String),
}

/// The members of an archive's package root, as a package's table files are
/// read from it.
#[derive(Clone, Debug)]
pub struct ArchiveFiles {
    /// The package root, `source/…`, without a trailing `/`.
    root: String,
    /// Every source member, by archive path.
    members: BTreeMap<String, Vec<u8>>,
}

impl PackageFiles for ArchiveFiles {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.members
            .get(&format!("{}/{path}", self.root))
            .cloned()
            .ok_or_else(|| "the file is not in the archive".to_owned())
    }
}

/// A read archive: its kind, identity and normalized packages.
#[derive(Clone, Debug)]
pub struct Archive {
    kind: PackageKind,
    identity: PackageIdentity,
    pkl_version: String,
    ruleset: Option<RuleSetPackage>,
    definitions: Vec<DefinitionPackage>,
    files: ArchiveFiles,
}

impl Archive {
    /// The kind the archived manifest states.
    pub fn kind(&self) -> PackageKind {
        self.kind
    }

    /// The package identity, equal in the metadata and the manifest.
    pub fn identity(&self) -> &PackageIdentity {
        &self.identity
    }

    /// The Pkl version the sources were evaluated with.
    pub fn pkl_version(&self) -> &str {
        &self.pkl_version
    }

    /// The rule set of a `ruleset` archive.
    pub fn ruleset(&self) -> Option<&RuleSetPackage> {
        self.ruleset.as_ref()
    }

    /// The definition documents, in the manifest's `definitionEntrypoints`
    /// order.
    pub fn definitions(&self) -> &[DefinitionPackage] {
        &self.definitions
    }

    /// The package root's members, from which table files were loaded.
    pub fn files(&self) -> &ArchiveFiles {
        &self.files
    }

    /// The rule set and the definition documents it carries.
    ///
    /// # Errors
    ///
    /// [`McsError::WrongKind`] for a `definitions` archive.
    pub fn into_ruleset(self) -> Result<(RuleSetPackage, Vec<DefinitionPackage>), McsError> {
        match self.ruleset {
            Some(ruleset) => Ok((ruleset, self.definitions)),
            None => Err(McsError::WrongKind {
                expected: PackageKind::RuleSet,
                found: self.kind,
            }),
        }
    }

    /// The definition documents of a `definitions` archive.
    ///
    /// # Errors
    ///
    /// [`McsError::WrongKind`] for a `ruleset` archive: its definitions are
    /// taken with its rule set ([`Archive::into_ruleset`]).
    pub fn into_definitions(self) -> Result<Vec<DefinitionPackage>, McsError> {
        match self.kind {
            PackageKind::Definitions => Ok(self.definitions),
            PackageKind::RuleSet => Err(McsError::WrongKind {
                expected: PackageKind::Definitions,
                found: self.kind,
            }),
        }
    }
}

/// Whether `bytes` open as an MCS archive does: a ZIP local header naming
/// `mimetype`, stored with the MCS media type.
pub fn is_archive(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        && bytes.get(26..30) == Some(&[8, 0, 0, 0])
        && bytes.get(30..38) == Some(b"mimetype".as_slice())
        && bytes.get(38..38 + MEDIA_TYPE.len()) == Some(MEDIA_TYPE.as_bytes())
}

/// Reads the archive file at `path`.
///
/// # Errors
///
/// [`McsError::Read`] when the path is a symbolic link, not a regular file
/// or unreadable, [`McsError::Limit`] when it is larger than
/// [`MAX_COMPRESSED`], and every error of [`read`].
pub fn read_file(path: &Path) -> Result<Archive, McsError> {
    let shown = path.display();
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| McsError::Read(format!("{shown}: {error}")))?;
    if !metadata.file_type().is_file() {
        return Err(McsError::Read(format!(
            "{shown}: an archive must be a regular file, not a link or directory"
        )));
    }
    if metadata.len() > MAX_COMPRESSED as u64 {
        return Err(McsError::Limit(format!(
            "the archive is larger than {MAX_COMPRESSED} bytes"
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| McsError::Read(format!("{shown}: {error}")))?;
    read(&bytes)
}

/// Reads an archive.
///
/// # Errors
///
/// The first refusal, as listed in the [crate documentation](crate).
pub fn read(bytes: &[u8]) -> Result<Archive, McsError> {
    let members = container::members(bytes)?;
    let mut data: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for member in members {
        data.insert(member.name, member.data);
    }
    if data.get("mimetype").map(Vec::as_slice) != Some(MEDIA_TYPE.as_bytes()) {
        return Err(McsError::Container("invalid mimetype payload".into()));
    }
    let metadata = Metadata::read(&data[METADATA])?;
    let declared = inventory(&metadata.inventory, &data)?;
    let manifest = source_topology(&metadata, &declared, &data)?;
    let identity = PackageIdentity {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        schema_version: manifest.schema_version.clone(),
    };
    if metadata.package != identity {
        return Err(McsError::IdentityMismatch {
            metadata: Box::new(metadata.package),
            manifest: Box::new(identity),
        });
    }
    normalized_mapping(&metadata, &declared, manifest.kind)?;
    static_checks(&metadata, &manifest, &declared, &data)?;
    let files = ArchiveFiles {
        root: metadata.package_root.clone(),
        members: declared
            .iter()
            .filter(|(name, role)| name.starts_with("source/") && *role == Role::Source)
            .map(|(name, _)| (name.clone(), data[name].clone()))
            .collect(),
    };
    let mut definitions = Vec::with_capacity(manifest.definition_entrypoints.len());
    for module in &manifest.definition_entrypoints {
        let member = &metadata.normalized[module];
        let mut package: DefinitionPackage = package(member, &data[member])?;
        if package.schema_version != identity.schema_version {
            return Err(McsError::Package {
                member: member.clone(),
                detail: "definition schema version disagrees with metadata".into(),
            });
        }
        load_definition_table_files(&mut package, &files)
            .map_err(|error| McsError::TableFile(format!("{member}: {error}")))?;
        definitions.push(package);
    }
    bind_definitions(&definitions)?;
    let ruleset = match &manifest.entrypoint {
        None => {
            let matches = definitions
                .iter()
                .filter(|package| {
                    package.package.id == identity.id && package.package.version == identity.version
                })
                .count();
            if matches != 1 {
                return Err(McsError::DefinitionsIdentity {
                    id: identity.id,
                    version: identity.version,
                    matches,
                });
            }
            None
        }
        Some(entrypoint) => Some(normalized_ruleset(
            &metadata,
            entrypoint,
            &identity,
            &definitions,
            &data,
            &files,
        )?),
    };
    Ok(Archive {
        kind: manifest.kind,
        identity,
        pkl_version: metadata.pkl_version,
        ruleset,
        definitions,
        files,
    })
}

/// What the MCS verifier checks by evaluating the sources, checked as far
/// as the archive shows it without evaluating them.
fn static_checks(
    metadata: &Metadata,
    manifest: &Manifest,
    declared: &[(String, Role)],
    data: &BTreeMap<String, Vec<u8>>,
) -> Result<(), McsError> {
    let pkl_version = std::str::from_utf8(&data["source/.pkl-version"])
        .map(str::trim)
        .unwrap_or_default();
    if pkl_version != metadata.pkl_version {
        return Err(McsError::Metadata(
            "source .pkl-version disagrees with metadata".into(),
        ));
    }
    let mut modules: BTreeSet<&str> = manifest
        .definition_entrypoints
        .iter()
        .map(String::as_str)
        .collect();
    modules.extend(manifest.entrypoint.as_deref());
    if metadata
        .normalized
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != modules
    {
        return Err(McsError::Metadata(
            "normalized mapping disagrees with manifest entrypoints".into(),
        ));
    }
    schema_member(metadata, manifest, declared)
}

/// The normalized rule set of a `ruleset` archive, with its table files.
fn normalized_ruleset(
    metadata: &Metadata,
    entrypoint: &str,
    identity: &PackageIdentity,
    definitions: &[DefinitionPackage],
    data: &BTreeMap<String, Vec<u8>>,
    files: &ArchiveFiles,
) -> Result<RuleSetPackage, McsError> {
    if metadata.normalized[entrypoint] != RULESET_MEMBER {
        return Err(McsError::Metadata(
            "invalid normalized ruleset mapping".into(),
        ));
    }
    let mut ruleset: RuleSetPackage = package(RULESET_MEMBER, &data[RULESET_MEMBER])?;
    if ruleset.package.id != identity.id
        || ruleset.package.version != identity.version
        || ruleset.schema_version != identity.schema_version
    {
        return Err(McsError::Package {
            member: RULESET_MEMBER.into(),
            detail: "the rule set's package identity disagrees with metadata".into(),
        });
    }
    for package in definitions {
        if !ruleset.definition_packages.contains(&package.package.id) {
            return Err(McsError::Binding(format!(
                "the archive carries definition package {:?}, which the rule set does \
                 not declare",
                package.package.id
            )));
        }
    }
    load_table_files(&mut ruleset, files)
        .map_err(|error| McsError::TableFile(format!("{RULESET_MEMBER}: {error}")))?;
    Ok(ruleset)
}

/// The definition packages rule sets are compiled against: every package
/// `supplied` (definitions archives, or JSON), then every package a rule-set
/// archive `carried` whose ID none supplied.
///
/// A package carried more than once, or carried and supplied, is kept once
/// when the copies are equal. Packages supplied twice are kept as given, so
/// the compiler refuses them as before.
///
/// # Errors
///
/// [`McsError::ConflictingDefinitions`] when a carried package differs from
/// another package of its ID.
pub fn resolve_definitions(
    supplied: Vec<DefinitionPackage>,
    carried: impl IntoIterator<Item = DefinitionPackage>,
) -> Result<Vec<DefinitionPackage>, McsError> {
    let mut packages = supplied;
    for package in carried {
        match packages
            .iter()
            .find(|known| known.package.id == package.package.id)
        {
            Some(known) if *known == package => {}
            Some(_) => return Err(McsError::ConflictingDefinitions(package.package.id)),
            None => packages.push(package),
        }
    }
    Ok(packages)
}

fn package<T: serde::de::DeserializeOwned>(member: &str, data: &[u8]) -> Result<T, McsError> {
    json::strict(data).map_err(|detail| McsError::Package {
        member: member.to_owned(),
        detail,
    })
}

/// The definition documents bind as one component space: package IDs,
/// rule-definition IDs and concept IDs are unique across them, and no ID
/// names components of two kinds.
fn bind_definitions(definitions: &[DefinitionPackage]) -> Result<(), McsError> {
    let mut packages = BTreeSet::new();
    let mut kinds: BTreeMap<&str, &str> = BTreeMap::new();
    for package in definitions {
        if !packages.insert(package.package.id.as_str()) {
            return Err(McsError::Binding(format!(
                "duplicate definition package {:?}",
                package.package.id
            )));
        }
    }
    for (kind, ids) in [
        (
            "rule definition",
            definitions
                .iter()
                .flat_map(|package| package.definitions.keys())
                .collect::<Vec<_>>(),
        ),
        (
            "object type",
            definitions
                .iter()
                .flat_map(|package| package.object_types.keys())
                .collect(),
        ),
        (
            "property",
            definitions
                .iter()
                .flat_map(|package| package.properties.keys())
                .collect(),
        ),
        (
            "property set",
            definitions
                .iter()
                .flat_map(|package| package.property_sets.keys())
                .collect(),
        ),
    ] {
        for id in ids {
            if let Some(known) = kinds.insert(id, kind) {
                return Err(McsError::Binding(if known == kind {
                    format!("duplicate {kind} {id:?}")
                } else {
                    format!("component id {id:?} is both {known} and {kind}")
                }));
            }
        }
    }
    Ok(())
}

/// A member's role in the inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Source,
    RuleSet,
    Definition,
}

/// The validated metadata member.
struct Metadata {
    package_root: String,
    manifest: String,
    pkl_version: String,
    package: PackageIdentity,
    normalized: BTreeMap<String, String>,
    /// The mapping's keys in the order written, which must be sorted.
    normalized_order: Vec<String>,
    inventory: Vec<Value>,
}

fn metadata_error(detail: &str) -> McsError {
    McsError::Metadata(detail.to_owned())
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

impl Metadata {
    fn read(data: &[u8]) -> Result<Self, McsError> {
        let value = json::strict_value(data).map_err(|error| metadata_error(&error))?;
        let Value::Object(object) = value else {
            return Err(metadata_error("metadata must be an object"));
        };
        let required = [
            "format",
            "formatVersion",
            "mediaType",
            "sourceRoot",
            "packageRoot",
            "manifest",
            "pklVersion",
            "package",
            "normalized",
            "inventory",
        ];
        if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
            return Err(metadata_error("metadata has missing or unknown keys"));
        }
        if string(&object, "format") != Some(FORMAT)
            || string(&object, "formatVersion") != Some(FORMAT_VERSION)
            || string(&object, "mediaType") != Some(MEDIA_TYPE)
            || string(&object, "sourceRoot") != Some("source")
        {
            return Err(metadata_error("invalid metadata constants"));
        }
        let package_root = match string(&object, "packageRoot") {
            Some(root)
                if root != "source" && root.starts_with("source/") && !root.ends_with('/') =>
            {
                root.to_owned()
            }
            _ => return Err(metadata_error("invalid metadata packageRoot")),
        };
        container::safe_name(&package_root)?;
        let manifest = match string(&object, "manifest") {
            Some(manifest) if manifest == format!("{package_root}/axioval.json") => {
                manifest.to_owned()
            }
            _ => return Err(metadata_error("invalid metadata manifest")),
        };
        container::safe_name(&manifest)?;
        let pkl_version = match string(&object, "pklVersion") {
            Some(version) if is_pkl_version(version) => version.to_owned(),
            _ => return Err(metadata_error("invalid metadata pklVersion")),
        };
        let package = match object.get("package") {
            Some(Value::Object(package))
                if package.len() == 3
                    && package.values().all(Value::is_string)
                    && string(package, "id").is_some_and(is_qualified_id)
                    && string(package, "version").is_some_and(is_semver)
                    && string(package, "schemaVersion").is_some_and(is_semver) =>
            {
                PackageIdentity {
                    id: string(package, "id").unwrap_or_default().to_owned(),
                    version: string(package, "version").unwrap_or_default().to_owned(),
                    schema_version: string(package, "schemaVersion")
                        .unwrap_or_default()
                        .to_owned(),
                }
            }
            _ => return Err(metadata_error("invalid metadata package")),
        };
        let (Some(Value::Object(mapping)), Some(Value::Array(inventory))) =
            (object.get("normalized"), object.get("inventory"))
        else {
            return Err(metadata_error("invalid metadata normalized/inventory"));
        };
        let mut normalized = BTreeMap::new();
        for (module, member) in mapping {
            let Some(member) = member.as_str() else {
                return Err(metadata_error("invalid normalized mapping"));
            };
            normalized.insert(module.clone(), member.to_owned());
        }
        let normalized_order =
            json::key_order(data, "normalized").map_err(|error| metadata_error(&error))?;
        Ok(Self {
            package_root,
            manifest,
            pkl_version,
            package,
            normalized,
            normalized_order,
            inventory: inventory.clone(),
        })
    }
}

/// The inventory, checked entry by entry against the members: every
/// member but `mimetype` and the metadata is declared once, with its
/// role, size and SHA-256, in path order.
fn inventory(
    entries: &[Value],
    data: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<(String, Role)>, McsError> {
    let mut declared: Vec<(String, Role)> = Vec::with_capacity(entries.len());
    let mut names = BTreeSet::new();
    for entry in entries {
        let Value::Object(entry) = entry else {
            return Err(metadata_error("invalid inventory entry"));
        };
        if entry.len() != 4
            || ["path", "role", "size", "sha256"]
                .iter()
                .any(|key| !entry.contains_key(*key))
        {
            return Err(metadata_error("invalid inventory entry"));
        }
        let (Some(name), Some(role), Some(size), Some(sha256)) = (
            string(entry, "path"),
            string(entry, "role"),
            entry.get("size").and_then(Value::as_u64),
            string(entry, "sha256"),
        ) else {
            return Err(metadata_error("invalid inventory types"));
        };
        let role = match role {
            "source" => Role::Source,
            "ruleset" => Role::RuleSet,
            "definition" => Role::Definition,
            _ => return Err(metadata_error("invalid inventory types")),
        };
        if size > MAX_FILE as u64 || !is_sha256(sha256) {
            return Err(metadata_error("invalid inventory types"));
        }
        container::safe_name(name)?;
        let expected = if name.starts_with("source/") {
            Some(Role::Source)
        } else if name == RULESET_MEMBER {
            Some(Role::RuleSet)
        } else if is_definition_member(name) {
            Some(Role::Definition)
        } else {
            None
        };
        if expected != Some(role) {
            return Err(metadata_error("invalid inventory member topology or role"));
        }
        if !names.insert(name.to_owned())
            || !data.contains_key(name)
            || name == "mimetype"
            || name == METADATA
        {
            return Err(metadata_error(
                "missing, duplicate, or reserved inventory member",
            ));
        }
        let bytes = &data[name];
        if bytes.len() as u64 != size || hex(&Sha256::digest(bytes)) != sha256 {
            return Err(metadata_error("inventory size/hash mismatch"));
        }
        declared.push((name.to_owned(), role));
    }
    if declared.windows(2).any(|pair| pair[0].0 > pair[1].0) {
        return Err(metadata_error("inventory must be sorted by path"));
    }
    if data.len() != declared.len() + 2 {
        return Err(metadata_error(
            "archive contains undeclared or missing payload",
        ));
    }
    Ok(declared)
}

fn role_of<'a>(declared: &'a [(String, Role)], name: &str) -> Option<&'a Role> {
    declared
        .binary_search_by(|(known, _)| known.as_str().cmp(name))
        .ok()
        .map(|index| &declared[index].1)
}

/// The validated archived manifest.
struct Manifest {
    kind: PackageKind,
    id: String,
    version: String,
    schema_version: String,
    entrypoint: Option<String>,
    definition_entrypoints: Vec<String>,
    schema: Option<String>,
}

/// The sources every archive carries (the manifest, `.pkl-version`,
/// `PklProject`, a licence at the source root), and the manifest read.
fn source_topology(
    metadata: &Metadata,
    declared: &[(String, Role)],
    data: &BTreeMap<String, Vec<u8>>,
) -> Result<Manifest, McsError> {
    let source = |name: &str| role_of(declared, name) == Some(&Role::Source);
    let licence = declared.iter().any(|(name, role)| {
        *role == Role::Source
            && name
                .strip_prefix("source/")
                .is_some_and(|rest| rest.starts_with("LICENSE") && !rest.contains('/'))
    });
    if !source(&metadata.manifest)
        || !source("source/.pkl-version")
        || !source("source/PklProject")
        || !licence
    {
        return Err(metadata_error("invalid source package topology"));
    }
    manifest(&data[&metadata.manifest])
}

fn manifest_error(detail: &str) -> McsError {
    McsError::Manifest(detail.to_owned())
}

/// The archived manifest, validated as `scripts/validate.py` validates a
/// package's `axioval.json`.
#[allow(clippy::too_many_lines)] // One pass, in the order `validate.py` checks.
fn manifest(data: &[u8]) -> Result<Manifest, McsError> {
    let value = json::strict_value(data).map_err(|error| manifest_error(&error))?;
    let Value::Object(object) = value else {
        return Err(manifest_error("manifest must be an object"));
    };
    // Anything but a definitions package is held to the rule-set contract,
    // so a missing or unknown kind still reports the rule-set fields.
    let definitions_only = string(&object, "kind") == Some("definitions");
    let mut required = vec![
        "manifestVersion",
        "kind",
        "id",
        "version",
        "schemaVersion",
        "definitionEntrypoints",
    ];
    if !definitions_only {
        required.push("entrypoint");
    }
    let mut allowed = required.clone();
    allowed.extend(["$schema", "description", "license"]);
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|key| !object.contains_key(*key))
        .collect();
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(McsError::Manifest(format!(
            "missing={missing:?}, unknown={unknown:?}"
        )));
    }
    if required
        .iter()
        .filter(|key| **key != "definitionEntrypoints")
        .any(|key| !object[*key].is_string())
    {
        return Err(manifest_error("manifest scalar fields must be strings"));
    }
    let entries: Vec<String> = match &object["definitionEntrypoints"] {
        Value::Array(entries) if !entries.is_empty() && entries.iter().all(Value::is_string) => {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        }
        _ => {
            return Err(manifest_error(
                "definitionEntrypoints must be non-empty unique strings",
            ));
        }
    };
    if entries.iter().collect::<BTreeSet<_>>().len() != entries.len() {
        return Err(manifest_error(
            "definitionEntrypoints must be non-empty unique strings",
        ));
    }
    let entrypoint = (!definitions_only).then(|| string(&object, "entrypoint").unwrap_or_default());
    if entrypoint.is_some_and(|entry| !is_pkl_entrypoint(entry))
        || entries.iter().any(|entry| !is_pkl_entrypoint(entry))
    {
        return Err(manifest_error(
            "entrypoints must be safe repository-relative .pkl paths",
        ));
    }
    let kind = match string(&object, "kind") {
        Some("ruleset") => Some(PackageKind::RuleSet),
        Some("definitions") => Some(PackageKind::Definitions),
        _ => None,
    };
    let id = string(&object, "id").unwrap_or_default();
    let version = string(&object, "version").unwrap_or_default();
    let (Some(kind), true) = (
        kind,
        string(&object, "manifestVersion") == Some(MANIFEST_VERSION)
            && string(&object, "schemaVersion") == Some(SCHEMA_VERSION)
            && is_qualified_id(id)
            && is_semver(version),
    ) else {
        return Err(manifest_error(
            "invalid manifest constants, package identity, or version",
        ));
    };
    if object
        .get("$schema")
        .is_some_and(|schema| !schema.is_string())
    {
        return Err(manifest_error("$schema must be a string"));
    }
    for field in ["description", "license"] {
        if object
            .get(field)
            .is_some_and(|value| value.as_str().is_none_or(str::is_empty))
        {
            return Err(McsError::Manifest(format!(
                "{field} must be a non-empty string"
            )));
        }
    }
    Ok(Manifest {
        kind,
        id: id.to_owned(),
        version: version.to_owned(),
        schema_version: SCHEMA_VERSION.to_owned(),
        entrypoint: entrypoint.map(str::to_owned),
        definition_entrypoints: entries,
        schema: string(&object, "$schema").map(str::to_owned),
    })
}

/// The normalized mapping: each declared module of the package root maps to
/// one distinct normalized member, every normalized member is mapped, and a
/// `ruleset` package maps exactly one rule set while a `definitions` package
/// maps none.
fn normalized_mapping(
    metadata: &Metadata,
    declared: &[(String, Role)],
    kind: PackageKind,
) -> Result<(), McsError> {
    let mut mapped = BTreeSet::new();
    let mut rulesets = 0;
    for module in &metadata.normalized_order {
        let member = &metadata.normalized[module];
        // MCS compares the extension case-sensitively.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        if !module.ends_with(".pkl") {
            return Err(metadata_error("invalid normalized mapping"));
        }
        container::safe_name(module)?;
        let source_module = format!("{}/{module}", metadata.package_root);
        if role_of(declared, &source_module) != Some(&Role::Source)
            || role_of(declared, member).is_none()
            || !mapped.insert(member.as_str())
        {
            return Err(metadata_error("missing or duplicate normalized mapping"));
        }
        match role_of(declared, member) {
            Some(Role::RuleSet) => {
                if member != RULESET_MEMBER {
                    return Err(metadata_error("invalid normalized ruleset path"));
                }
                rulesets += 1;
            }
            Some(Role::Definition) => {}
            _ => {
                return Err(metadata_error(
                    "normalized mapping points to a non-normalized role",
                ));
            }
        }
    }
    if metadata
        .normalized_order
        .windows(2)
        .any(|pair| pair[0] > pair[1])
    {
        return Err(metadata_error(
            "normalized mapping must be sorted by source module",
        ));
    }
    let normalized: BTreeSet<&str> = declared
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| name.starts_with("normalized/"))
        .collect();
    if mapped != normalized {
        return Err(metadata_error("incomplete normalized mapping"));
    }
    match (kind, rulesets) {
        (PackageKind::RuleSet, 1) | (PackageKind::Definitions, 0) => Ok(()),
        (PackageKind::RuleSet, _) => Err(McsError::RuleSetMissing),
        (PackageKind::Definitions, _) => Err(McsError::DefinitionsCarryRuleSet),
    }
}

/// The manifest's `$schema` names a source member relative to the manifest.
fn schema_member(
    metadata: &Metadata,
    manifest: &Manifest,
    declared: &[(String, Role)],
) -> Result<(), McsError> {
    let invalid = || manifest_error("manifest schema must be a relative archive member");
    let reference = manifest.schema.as_deref().ok_or_else(invalid)?;
    if reference.contains(':') || reference.starts_with(['/', '\\']) {
        return Err(invalid());
    }
    // `source/<package root>/` joined with the reference, `..` resolved
    // lexically; it must stay inside `source/`.
    let mut path: Vec<&str> = metadata.package_root.split('/').collect();
    for segment in reference.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                path.pop();
                if path.is_empty() {
                    return Err(invalid());
                }
            }
            segment => path.push(segment),
        }
    }
    let member = path.join("/");
    if path.len() < 2 || role_of(declared, &member) != Some(&Role::Source) {
        return Err(invalid());
    }
    Ok(())
}

fn is_definition_member(name: &str) -> bool {
    name.strip_prefix("normalized/definitions/")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|stem| {
            stem.len() == 16
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// `[0-9]+\.[0-9]+\.[0-9]+`.
fn is_pkl_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// `^[a-z][a-z0-9+.-]*:.+$`.
fn is_qualified_id(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let mut scheme = scheme.bytes();
    scheme.next().is_some_and(|b| b.is_ascii_lowercase())
        && scheme.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
        && !rest.is_empty()
        && !rest.contains('\n')
}

/// The semantic version `SEMVER` of the MCS scripts: three numbers without
/// leading zeros, an optional pre-release and build of `[0-9A-Za-z.-]`.
fn is_semver(value: &str) -> bool {
    let identifiers = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    };
    let (rest, build) = match value.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (value, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers.iter().all(|number| {
            !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
                && (*number == "0" || !number.starts_with('0'))
        })
        && pre.is_none_or(identifiers)
        && build.is_none_or(identifiers)
}

/// `^(?!/)(?!.*(?:^|/)\.\.(?:/|$)).+\.pkl$`: a relative `.pkl` path with no
/// `..` segment.
fn is_pkl_entrypoint(value: &str) -> bool {
    !value.contains('\n')
        && !value.starts_with('/')
        && !value.split('/').any(|segment| segment == "..")
        && value
            .strip_suffix(".pkl")
            .is_some_and(|stem| !stem.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_ids_follow_the_mcs_patterns() {
        for good in ["0.1.0", "1.2.3-rc.1", "1.2.3+build.5", "10.0.0-a-b+c"] {
            assert!(is_semver(good), "{good}");
        }
        for bad in ["01.0.0", "1.0", "1.0.0-", "1.0.0+", "1.0.0-a_b", "v1.0.0"] {
            assert!(!is_semver(bad), "{bad}");
        }
        assert!(is_qualified_id("axioval:example.ruleset"));
        for bad in [
            "Axioval:x",
            "axioval:",
            "ax_ioval:x",
            "axioval",
            "axioval:a\nb",
        ] {
            assert!(!is_qualified_id(bad), "{bad:?}");
        }
        assert!(is_pkl_entrypoint("rules/ruleset.pkl"));
        for bad in [
            "/ruleset.pkl",
            "../ruleset.pkl",
            "a/../b.pkl",
            ".pkl",
            "a.json",
        ] {
            assert!(!is_pkl_entrypoint(bad), "{bad}");
        }
        assert!(is_pkl_version("0.32.1"));
        assert!(!is_pkl_version("0.32"));
    }
}
