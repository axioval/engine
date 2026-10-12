//! Cadastral parcels and public traffic areas (#319), read from a GML
//! subset of the German cadastre's exchange (ALKIS, as NAS writes it).
//!
//! The features read, each one zone:
//!
//! | feature              | kind     | id                       |
//! |----------------------|----------|--------------------------|
//! | `AX_Flurstueck`      | `parcel` | `flurstueckskennzeichen` |
//! | `AX_Strassenverkehr` | `street` | its `gml:id`             |
//! | `AX_Weg`             | `way`    | its `gml:id`             |
//! | `AX_Platz`           | `square` | its `gml:id`             |
//!
//! Each one's geometry is its `position` (an area, see [`super::gml`]).
//! The exchange states no accuracy of its boundaries, so the caller states
//! it. Every other feature is refused by its name, as is a feature without
//! its id or geometry: a cadastre read in part is never mistaken for the
//! whole one.

use super::gml::{self, Node};
use super::{ContextZone, ContextZones, ZoneShape};

/// The features read: their name, the kind of zone and whether the id is
/// the parcel's identifier (`flurstueckskennzeichen`) rather than the
/// feature's `gml:id`.
const FEATURES: &[(&str, &str, bool)] = &[
    ("AX_Flurstueck", "parcel", true),
    ("AX_Strassenverkehr", "street", false),
    ("AX_Weg", "way", false),
    ("AX_Platz", "square", false),
];

/// The names of the features read, for a refusal.
fn read_names() -> String {
    FEATURES
        .iter()
        .map(|(name, ..)| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ContextZones {
    /// Reads the parcels and public traffic areas of a cadastral GML
    /// document, the file `name`, whose boundaries are stated within
    /// `accuracy_metres`: the subset of the module documentation. The
    /// reference system is the one every geometry states (`srsName`, on
    /// it or above it, else on the document's envelope), read easting
    /// first.
    ///
    /// # Errors
    ///
    /// Why the document is refused, naming the feature: a feature or
    /// geometry outside the subset, a feature without its id or
    /// geometry, a geometry that is not an area, no reference system or
    /// several, one whose axis order is not known, two zones of one id,
    /// and an accuracy not finite and non-negative.
    pub fn read_cadastre_gml(
        bytes: &[u8],
        name: &str,
        accuracy_metres: f64,
    ) -> Result<Self, String> {
        let root = gml::parse(bytes, "cadastral GML")?;
        Self::cadastre_from(&root, bytes, name, accuracy_metres)
    }

    /// [`Self::read_cadastre_gml`] of a document already parsed.
    pub(super) fn cadastre_from(
        root: &Node,
        bytes: &[u8],
        name: &str,
        accuracy_metres: f64,
    ) -> Result<Self, String> {
        let envelope = gml::envelope_reference(root);
        let mut zones = Vec::new();
        let mut references = Vec::new();
        for (index, feature) in gml::features(root).into_iter().enumerate() {
            let number = index + 1;
            let refuse = |why: String| format!("feature {number} (`{}`): {why}", feature.name);
            let &(_, kind, by_identifier) = FEATURES
                .iter()
                .find(|(read, ..)| *read == feature.name)
                .ok_or_else(|| {
                    refuse(format!(
                        "it is not read; the cadastral reader reads {}",
                        read_names()
                    ))
                })?;
            let id = id(feature, by_identifier).map_err(refuse)?;
            let position = feature
                .child("position")
                .ok_or_else(|| refuse(format!("`{id}` states no `position`")))?;
            let geometry =
                gml::geometry(position, None).map_err(|why| refuse(format!("`{id}`: {why}")))?;
            if !matches!(geometry.shape, ZoneShape::Area(_)) {
                return Err(refuse(format!("`{id}`: its `position` is not an area")));
            }
            references.push(geometry.srs);
            zones.push(
                ContextZone::try_new(id, kind, geometry.shape, accuracy_metres, number)
                    .map_err(refuse)?,
            );
        }
        if zones.is_empty() {
            return Err(format!(
                "it states no feature the cadastral reader reads ({})",
                read_names()
            ));
        }
        let reference = gml::reference(references, envelope)?;
        Self::new(
            name,
            crate::table_files::sha256_hex(bytes),
            reference,
            zones,
        )
        .map(|zones| zones.read_as("cadastre-gml"))
    }
}

/// The zone id of a feature: its parcel identifier, or its `gml:id`.
fn id(feature: &Node, by_identifier: bool) -> Result<String, String> {
    if by_identifier {
        let identifier = feature
            .child("flurstueckskennzeichen")
            .map(Node::text)
            .filter(|text| !text.is_empty())
            .ok_or("it states no `flurstueckskennzeichen`")?;
        return Ok(identifier.to_owned());
    }
    feature
        .attribute("id")
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "it states no `gml:id`".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_zones::ZoneReference;

    fn parcel(id: &str, ring: &str) -> String {
        format!(
            "<gml:featureMember><AX_Flurstueck gml:id=\"F{id}\">\
             <flurstueckskennzeichen>{id}</flurstueckskennzeichen>\
             <position><gml:Polygon><gml:exterior><gml:LinearRing>\
             <gml:posList>{ring}</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon>\
             </position></AX_Flurstueck></gml:featureMember>"
        )
    }

    fn document(srs: &str, members: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <AX_Bestandsdatenauszug xmlns:gml=\"http://www.opengis.net/gml/3.2\">\
             <enthaelt><wfs:FeatureCollection xmlns:wfs=\"http://www.opengis.net/wfs\">\
             <gml:boundedBy><gml:Envelope srsName=\"{srs}\"/></gml:boundedBy>\
             {members}</wfs:FeatureCollection></enthaelt></AX_Bestandsdatenauszug>"
        )
    }

    const STREET: &str = "<gml:featureMember><AX_Strassenverkehr gml:id=\"S1\"><position>\
         <gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 -10 30 -10 30 0 0 0 0 -10\
         </gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></position>\
         </AX_Strassenverkehr></gml:featureMember>";

    #[test]
    fn parcels_and_a_street_are_read_with_their_identifiers_and_kinds() {
        let text = document(
            "urn:adv:crs:ETRS89_UTM32",
            &format!(
                "{}{STREET}",
                parcel("052345001000120000__", "0 0 30 0 30 20 0 20 0 0")
            ),
        );
        let zones = ContextZones::read_cadastre_gml(text.as_bytes(), "alkis.xml", 0.03).unwrap();
        assert_eq!(zones.format(), Some("cadastre-gml"));
        assert_eq!(
            zones.reference(),
            &ZoneReference::Map("urn:adv:crs:ETRS89_UTM32".into())
        );
        let read: Vec<_> = zones
            .zones()
            .iter()
            .map(|zone| (zone.id(), zone.kind(), zone.feature()))
            .collect();
        assert_eq!(
            read,
            [("052345001000120000__", "parcel", 1), ("S1", "street", 2)]
        );
        assert!((zones.zones()[0].accuracy_metres() - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn what_the_reader_does_not_read_is_refused_by_name() {
        let building = "<gml:featureMember><AX_Gebaeude gml:id=\"G1\"/></gml:featureMember>";
        let line = "<gml:featureMember><AX_Weg gml:id=\"W1\"><position><gml:LineString>\
                    <gml:posList>0 0 1 1</gml:posList></gml:LineString></position></AX_Weg>\
                    </gml:featureMember>";
        let ring = "0 0 30 0 30 20 0 20 0 0";
        for (text, why) in [
            (
                document("EPSG:25832", building),
                "`AX_Gebaeude`): it is not read",
            ),
            (document("EPSG:25832", line), "not an area"),
            (
                document("EPSG:31467", &parcel("1", ring)),
                "is not read: the GML subset reads ETRS89",
            ),
            (
                document(
                    "EPSG:25832",
                    &parcel("1", ring).replace("flurstueckskennzeichen", "x"),
                ),
                "no `flurstueckskennzeichen`",
            ),
            (
                document(
                    "EPSG:25832",
                    &format!("{}{}", parcel("1", ring), parcel("1", ring)),
                ),
                "two zones",
            ),
            (document("EPSG:25832", ""), "states no feature"),
            ("<a><b></a>".to_owned(), "malformed"),
        ] {
            let error = ContextZones::read_cadastre_gml(text.as_bytes(), "a.xml", 0.0).unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
        let text = document("EPSG:25832", &parcel("1", ring));
        assert!(
            ContextZones::read_cadastre_gml(text.as_bytes(), "a.xml", -1.0)
                .unwrap_err()
                .contains("accuracy")
        );
    }
}
