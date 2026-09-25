//! The one IFC release a session reads, fixed from the file header.
//!
//! Every schema question in this crate (entity ancestry, relationship end
//! slots, attribute optionality, which relationship types exist) is answered
//! from the table of the release the file declares. IFC2X3 and IFC4 differ in
//! exactly these facts: `IfcRelSpaceBoundary.RelatedBuildingElement` is
//! optional in IFC2X3 and required in IFC4, `IfcZone` is an `IfcGroup` in
//! IFC2X3 and an `IfcSystem` in IFC4. Reading one release with the other's
//! table gives confident wrong answers, so the release is chosen once, here,
//! and passed to every service rather than looked up per call.

use ifc_schema::{Schema, SchemaVersion, ifc2x3, ifc4};

/// Type system of the IFC2x3 TC1 release, as declared by `openbim.ifc`.
///
/// buildingSMART publishes no bSDD dictionary for IFC2x3, so `openbim.ifc`
/// identifies the release by its normative documentation root. Package
/// concepts must name IFC2x3 entities under exactly this identifier to bind to
/// IFC2X3 files; an IFC4 name does not bind to IFC2X3 data.
pub const IFC2X3_TYPE_SYSTEM: &str =
    "https://standards.buildingsmart.org/IFC/RELEASE/IFC2x3/TC1/HTML/";

/// Type system of the IFC4 ADD2 TC1 release, as declared by `openbim.ifc`.
///
/// Package concepts bind to data from this adapter only through an external
/// name in exactly this type system. It is the release's semantic identifier,
/// not a transport or documentation URL, and it differs per IFC release.
pub const IFC4_TYPE_SYSTEM: &str = "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4";

/// The release a session is bound to.
///
/// Only releases the exact property resolver also reads are constructible:
/// a session whose properties would all be refused is not a useful session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Release {
    /// Normative entity and type table of the release.
    pub(crate) schema: &'static Schema,
    /// Header token recorded on the snapshot and used in messages.
    pub(crate) label: &'static str,
    /// Semantic type-system identity declared on the snapshot.
    pub(crate) type_system: &'static str,
}

impl Release {
    /// The release a header declares, when it declares exactly one supported one.
    pub(crate) fn from_header(schemas: &[String]) -> Option<Self> {
        let [token] = schemas else {
            return None;
        };
        match SchemaVersion::from_header_token(token)? {
            SchemaVersion::Ifc2x3 => Some(Self {
                schema: ifc2x3(),
                label: "IFC2X3",
                type_system: IFC2X3_TYPE_SYSTEM,
            }),
            SchemaVersion::Ifc4 => Some(Self {
                schema: ifc4(),
                label: "IFC4",
                type_system: IFC4_TYPE_SYSTEM,
            }),
            // IFC4X3 has a schema table but no exact property resolution yet;
            // refuse rather than half-support. Named, not wildcarded, so a
            // release added upstream is a compile error here, not a silent refusal.
            SchemaVersion::Ifc4x3 => None,
        }
    }
}
