//! The regulating areas and lines of a zoning plan (#321), read from a
//! subset of `XPlanGML` (an `XPlanAuszug` of versions 5 and 6), with the
//! values its areas state.
//!
//! The features read, each one zone by its `gml:id` with its `position`:
//!
//! | feature                              | kind             | shape |
//! |--------------------------------------|------------------|-------|
//! | `BP_BaugebietsTeilFlaeche`           | `use-area`       | area  |
//! | `BP_UeberbaubareGrundstuecksFlaeche` | `buildable-area` | area  |
//! | `BP_BaugrenzeLinie`                  | `building-limit` | line  |
//! | `BP_BaulinieLinie`                   | `building-line`  | line  |
//!
//! The plan and its sections (`BP_Plan`, `BP_Bereich`), text sections and
//! presentation objects state no zone and are passed over; every other
//! feature is refused by its name.
//!
//! An area's stated values, by the neutral keys of
//! [`axioval_ir::measured`]:
//!
//! - `GRZ` and `GRZmax` the greatest coverage ratio, `GRZmin` the least;
//!   `GFZ`, `GFZmax` and `GFZmin` the floor area ratio alike;
//! - `Z` and `Zmax` the most full storeys, `Zmin` the least, `Zzwingend`
//!   the number required;
//! - `DN` and `DNmax` the greatest roof pitch, `DNmin` the least,
//!   `DNZwingend` the pitch required, on the feature or in its one
//!   `dachgestaltung`;
//! - each `hoehenangabe` whose `bezugspunkt` is the eaves (`1000`), the
//!   ridge (`2000`) or the top (`3000`): `h` and `hMax` the greatest
//!   height, `hMin` the least, `hZwingend` the height required. Only a
//!   height above the terrain (`hoehenbezug` `2000`) is read; one above
//!   another reference, or none, leaves its keys unread, never read as
//!   above the terrain.
//!
//! A key stated twice with different values (`GRZ` and `GRZmax`, or the
//! pitches of several roof forms) is unread, naming both; a length in a
//! unit other than metres, an angle in one other than degrees or radians,
//! and a value that is no number are refused.

use std::collections::BTreeMap;

use super::gml::{self, Node};
use super::{ContextZone, ContextZones, ZoneShape, ZoneValue};

/// The features read: their name, the kind of zone and whether it is an
/// area stating values (else a line).
const FEATURES: &[(&str, &str, bool)] = &[
    ("BP_BaugebietsTeilFlaeche", "use-area", true),
    ("BP_UeberbaubareGrundstuecksFlaeche", "buildable-area", true),
    ("BP_BaugrenzeLinie", "building-limit", false),
    ("BP_BaulinieLinie", "building-line", false),
];

/// Features stating no zone, passed over.
const PASSED_OVER: &[&str] = &[
    "BP_Plan",
    "BP_Bereich",
    "BP_TextAbschnitt",
    "XP_TextAbschnitt",
    "XP_Praesentationsobjekt",
    "XP_PPO",
    "XP_PTO",
    "XP_LPO",
    "XP_LTO",
    "XP_FPO",
    "XP_TPO",
    "XP_Nutzungsschablone",
    "XP_Rasterdarstellung",
];

/// The plain numbers read: the element, the key it states and which
/// bound it is.
const NUMBERS: &[(&str, &str)] = &[
    ("GRZ", "coverage_ratio_max"),
    ("GRZmax", "coverage_ratio_max"),
    ("GRZmin", "coverage_ratio_min"),
    ("GFZ", "floor_area_ratio_max"),
    ("GFZmax", "floor_area_ratio_max"),
    ("GFZmin", "floor_area_ratio_min"),
    ("Z", "storeys_max"),
    ("Zmax", "storeys_max"),
    ("Zmin", "storeys_min"),
    ("Zzwingend", "storeys"),
];

/// The roof pitches read.
const PITCHES: &[(&str, &str)] = &[
    ("DN", "roof_pitch_max"),
    ("DNmax", "roof_pitch_max"),
    ("DNmin", "roof_pitch_min"),
    ("DNZwingend", "roof_pitch"),
    ("DNzwingend", "roof_pitch"),
];

/// The heights read, by the code of their reference point.
const HEIGHT_POINTS: &[(&str, &str)] = &[("1000", "eaves"), ("2000", "ridge"), ("3000", "top")];

/// The heights of one reference point read: the element and its key's
/// suffix.
const HEIGHTS: &[(&str, &str)] = &[
    ("h", "_max"),
    ("hMax", "_max"),
    ("hMin", "_min"),
    ("hZwingend", ""),
];

/// The height reference read: above the terrain.
const ABOVE_TERRAIN: &str = "2000";

fn read_names() -> String {
    FEATURES
        .iter()
        .map(|(name, ..)| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ContextZones {
    /// Reads the regulating areas and lines of an `XPlanGML` document, the
    /// file `name`, whose boundaries are stated within `accuracy_metres`,
    /// with the values its areas state: the subset of the module
    /// documentation. The reference system is the one every geometry
    /// states (or the document's envelope), read easting first.
    ///
    /// # Errors
    ///
    /// Why the document is refused, naming the feature: a root other than
    /// `XPlanAuszug`, a feature or geometry outside the subset, a feature
    /// without its `gml:id` or `position`, a geometry of the wrong shape,
    /// a value that is no number or of a unit not read, no reference
    /// system or several, one whose axis order is not known, two zones of
    /// one id, and an accuracy not finite and non-negative.
    pub fn read_xplan_gml(bytes: &[u8], name: &str, accuracy_metres: f64) -> Result<Self, String> {
        let root = gml::parse(bytes, "XPlanGML")?;
        if root.name != "XPlanAuszug" {
            return Err(format!(
                "the root element is `{}`, not an `XPlanAuszug`",
                root.name
            ));
        }
        Self::xplan_from(&root, bytes, name, accuracy_metres)
    }

    /// [`Self::read_xplan_gml`] of a document already parsed.
    pub(super) fn xplan_from(
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
            if PASSED_OVER.contains(&feature.name.as_str()) {
                continue;
            }
            let refuse = |why: String| format!("feature {number} (`{}`): {why}", feature.name);
            let &(_, kind, area) = FEATURES
                .iter()
                .find(|(read, ..)| *read == feature.name)
                .ok_or_else(|| {
                    refuse(format!(
                        "it is not read; the zoning-plan reader reads {}",
                        read_names()
                    ))
                })?;
            let id = feature
                .attribute("id")
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| refuse("it states no `gml:id`".into()))?
                .to_owned();
            let position = feature
                .child("position")
                .ok_or_else(|| refuse(format!("`{id}` states no `position`")))?;
            let geometry =
                gml::geometry(position, None).map_err(|why| refuse(format!("`{id}`: {why}")))?;
            if matches!(geometry.shape, ZoneShape::Area(_)) != area {
                return Err(refuse(format!(
                    "`{id}`: its `position` is not {}",
                    if area { "an area" } else { "a line" }
                )));
            }
            let values = if area {
                stated_values(feature).map_err(|why| refuse(format!("`{id}`: {why}")))?
            } else {
                BTreeMap::new()
            };
            references.push(geometry.srs);
            zones.push(
                ContextZone::try_new(id, kind, geometry.shape, accuracy_metres, number)
                    .map_err(refuse)?
                    .with_values(values),
            );
        }
        if zones.is_empty() {
            return Err(format!(
                "it states no feature the zoning-plan reader reads ({})",
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
        .map(|zones| zones.read_as("xplan-gml"))
    }
}

/// The statements of each key, as they are found.
#[derive(Default)]
struct Statements {
    found: BTreeMap<String, Vec<(f64, bool, String)>>,
    unread: BTreeMap<String, String>,
}

impl Statements {
    fn state(&mut self, key: String, value: f64, degrees: bool, statement: String) {
        self.found
            .entry(key)
            .or_default()
            .push((value, degrees, statement));
    }

    fn values(self) -> BTreeMap<String, ZoneValue> {
        let mut values: BTreeMap<String, ZoneValue> = self
            .found
            .into_iter()
            .map(|(key, statements)| {
                let (value, degrees, statement) = statements[0].clone();
                #[allow(clippy::float_cmp)] // Equal as stated.
                let differ = statements
                    .iter()
                    .any(|(other, other_degrees, _)| *other != value || *other_degrees != degrees);
                let read = if differ {
                    ZoneValue::Unread {
                        why: format!(
                            "`{key}` is stated differently by {}",
                            statements
                                .iter()
                                .map(|(value, _, statement)| format!("`{statement}` {value}"))
                                .collect::<Vec<_>>()
                                .join(" and ")
                        ),
                    }
                } else {
                    ZoneValue::Stated {
                        value,
                        degrees,
                        statement,
                    }
                };
                (key, read)
            })
            .collect();
        for (key, why) in self.unread {
            values.insert(key, ZoneValue::Unread { why });
        }
        values
    }
}

/// A number stated as an element's text.
fn number(node: &Node) -> Result<f64, String> {
    node.text()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            format!(
                "`{}` states `{}`, which is no number",
                node.name,
                node.text()
            )
        })
}

/// An angle and whether it is in degrees, by its `uom`.
fn angle(node: &Node) -> Result<(f64, bool), String> {
    let degrees = match node.attribute("uom").map(str::trim) {
        Some("grad" | "deg" | "degree" | "°" | "urn:adv:uom:grad") => true,
        Some("rad") => false,
        Some(other) => {
            return Err(format!(
                "`{}` is stated in `{other}`; angles are read in degrees (`grad`) or `rad`",
                node.name
            ));
        }
        None => return Err(format!("`{}` states no unit (`uom`)", node.name)),
    };
    Ok((number(node)?, degrees))
}

/// A length in metres, by its `uom`.
fn length(node: &Node) -> Result<f64, String> {
    match node.attribute("uom").map(str::trim) {
        Some("m" | "urn:adv:uom:m") => number(node),
        Some(other) => Err(format!(
            "`{}` is stated in `{other}`; lengths are read in metres (`m`)",
            node.name
        )),
        None => Err(format!("`{}` states no unit (`uom`)", node.name)),
    }
}

/// The values an area feature states.
fn stated_values(feature: &Node) -> Result<BTreeMap<String, ZoneValue>, String> {
    let mut statements = Statements::default();
    for &(element, key) in NUMBERS {
        for node in feature.children_named(element) {
            statements.state(key.to_owned(), number(node)?, false, element.to_owned());
        }
    }
    let roofs: Vec<&Node> = feature
        .children_named("dachgestaltung")
        .flat_map(|roof| &roof.children)
        .collect();
    for (holder, prefix) in std::iter::once((feature, String::new())).chain(
        roofs
            .iter()
            .enumerate()
            .map(|(index, roof)| (*roof, format!("dachgestaltung[{}].", index + 1))),
    ) {
        for &(element, key) in PITCHES {
            for node in holder.children_named(element) {
                let (value, degrees) = angle(node)?;
                statements.state(key.to_owned(), value, degrees, format!("{prefix}{element}"));
            }
        }
    }
    for (index, wrapper) in feature.children_named("hoehenangabe").enumerate() {
        for height in &wrapper.children {
            let point = height.child("bezugspunkt").map(Node::text);
            let Some(&(code, name)) =
                point.and_then(|point| HEIGHT_POINTS.iter().find(|(code, _)| *code == point))
            else {
                // A height of another point, or a vertical restriction of
                // the area itself: no eaves, ridge or top height.
                continue;
            };
            let statement = |element: &str| {
                format!("hoehenangabe[{}].{element} (bezugspunkt {code})", index + 1)
            };
            let reference = height.child("hoehenbezug").map(Node::text);
            for &(element, suffix) in HEIGHTS {
                let Some(node) = height.child(element) else {
                    continue;
                };
                let key = format!("{name}_height{suffix}");
                if reference != Some(ABOVE_TERRAIN) {
                    let why = match reference {
                        Some(other) => format!(
                            "`{}` is a height above the reference `{other}`; only heights above \
                             the terrain (`hoehenbezug` {ABOVE_TERRAIN}) are read",
                            statement(element)
                        ),
                        None => format!(
                            "`{}` states no reference (`hoehenbezug`), so it is not read as a \
                             height above the terrain",
                            statement(element)
                        ),
                    };
                    statements.unread.insert(key, why);
                    continue;
                }
                statements.state(key, length(node)?, false, statement(element));
            }
        }
    }
    Ok(statements.values())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::measured::{ZONE_ANGLES, ZONE_LENGTHS, ZONE_NUMBERS};

    fn area(id: &str, body: &str) -> String {
        format!(
            "<gml:featureMember><xplan:BP_BaugebietsTeilFlaeche gml:id=\"{id}\">\
             <xplan:position><gml:Polygon srsName=\"EPSG:25832\"><gml:exterior><gml:LinearRing>\
             <gml:posList>0 0 40 0 40 30 0 30 0 0</gml:posList></gml:LinearRing></gml:exterior>\
             </gml:Polygon></xplan:position>{body}</xplan:BP_BaugebietsTeilFlaeche>\
             </gml:featureMember>"
        )
    }

    fn plan(members: &str) -> String {
        format!(
            "<xplan:XPlanAuszug xmlns:xplan=\"http://www.xplanung.de/xplangml/6/0\" \
             xmlns:gml=\"http://www.opengis.net/gml/3.2\" gml:id=\"A\">\
             <gml:featureMember><xplan:BP_Plan gml:id=\"P\"/></gml:featureMember>{members}\
             </xplan:XPlanAuszug>"
        )
    }

    fn height(point: &str, reference: &str, element: &str, value: &str) -> String {
        format!(
            "<xplan:hoehenangabe><xplan:XP_Hoehenangabe>{reference}\
             <xplan:bezugspunkt>{point}</xplan:bezugspunkt>\
             <xplan:{element} uom=\"m\">{value}</xplan:{element}></xplan:XP_Hoehenangabe>\
             </xplan:hoehenangabe>"
        )
    }

    const TERRAIN: &str = "<xplan:hoehenbezug>2000</xplan:hoehenbezug>";

    fn read(members: &str) -> Result<ContextZones, String> {
        ContextZones::read_gml(plan(members).as_bytes(), "plan.gml", 0.1)
    }

    #[test]
    #[allow(clippy::float_cmp)] // Stated values, compared as stated.
    fn a_use_area_states_its_ratios_pitches_and_heights() {
        let body = format!(
            "<xplan:GRZ>0.4</xplan:GRZ><xplan:GFZ>0.8</xplan:GFZ><xplan:Zzwingend>2</xplan:Zzwingend>\
             <xplan:dachgestaltung><xplan:BP_Dachgestaltung>\
             <xplan:DNmin uom=\"grad\">30</xplan:DNmin><xplan:DNmax uom=\"grad\">45</xplan:DNmax>\
             </xplan:BP_Dachgestaltung></xplan:dachgestaltung>{}{}",
            height("1000", TERRAIN, "hMax", "6.5"),
            height(
                "2000",
                "<xplan:hoehenbezug>1000</xplan:hoehenbezug>",
                "h",
                "62.0"
            ),
        );
        let zones = read(&area("WA1", &body)).unwrap();
        assert_eq!(zones.format(), Some("xplan-gml"));
        let zone = &zones.zones()[0];
        assert_eq!(
            (zone.id(), zone.kind(), zone.feature()),
            ("WA1", "use-area", 2)
        );
        let value = |key: &str| zone.values().get(key).cloned();
        assert_eq!(
            value("coverage_ratio_max"),
            Some(ZoneValue::Stated {
                value: 0.4,
                degrees: false,
                statement: "GRZ".into()
            })
        );
        assert!(matches!(value("storeys"), Some(ZoneValue::Stated { value, .. }) if value == 2.0));
        assert!(matches!(
            value("roof_pitch_min"),
            Some(ZoneValue::Stated { value, degrees: true, .. }) if value == 30.0
        ));
        assert!(matches!(
            value("eaves_height_max"),
            Some(ZoneValue::Stated { value, .. }) if value == 6.5
        ));
        assert!(
            matches!(value("ridge_height_max"), Some(ZoneValue::Unread { why }) if why.contains("reference `1000`"))
        );
        assert_eq!(value("coverage_ratio_min"), None, "stated nowhere: null");
        for key in zone.values().keys() {
            assert!(
                [ZONE_NUMBERS, ZONE_LENGTHS, ZONE_ANGLES]
                    .iter()
                    .any(|keys| keys.contains(&key.as_str())),
                "{key}"
            );
        }
    }

    #[test]
    fn a_value_stated_twice_differently_is_unread() {
        let zones = read(&area(
            "WA1",
            "<xplan:GRZ>0.4</xplan:GRZ><xplan:GRZmax>0.5</xplan:GRZmax><xplan:GFZ>0.8</xplan:GFZ>\
             <xplan:GFZmax>0.8</xplan:GFZmax>",
        ))
        .unwrap();
        let values = zones.zones()[0].values();
        assert!(
            matches!(&values["coverage_ratio_max"], ZoneValue::Unread { why } if why.contains("`GRZ` 0.4 and `GRZmax` 0.5"))
        );
        assert!(matches!(
            &values["floor_area_ratio_max"],
            ZoneValue::Stated { .. }
        ));
    }

    #[test]
    fn limits_and_lines_are_read_and_the_rest_refused_by_name() {
        let line = |kind: &str| {
            format!(
                "<gml:featureMember><xplan:{kind} gml:id=\"{kind}1\"><xplan:position>\
                 <gml:LineString srsName=\"EPSG:25832\"><gml:posList>0 0 10 0</gml:posList>\
                 </gml:LineString></xplan:position></xplan:{kind}></gml:featureMember>"
            )
        };
        let zones = read(&format!(
            "{}{}",
            line("BP_BaugrenzeLinie"),
            line("BP_BaulinieLinie")
        ))
        .unwrap();
        let kinds: Vec<_> = zones.zones().iter().map(ContextZone::kind).collect();
        assert_eq!(kinds, ["building-limit", "building-line"]);
        for (members, why) in [
            (
                line("BP_GruenFlaeche"),
                "`BP_GruenFlaeche`): it is not read",
            ),
            (
                line("BP_BaugebietsTeilFlaeche"),
                "its `position` is not an area",
            ),
            (
                area("WA1", "<xplan:DN uom=\"%\">30</xplan:DN>"),
                "angles are read in degrees",
            ),
            (
                area("WA1", &height("1000", TERRAIN, "hMax", "x")),
                "which is no number",
            ),
            (String::new(), "states no feature"),
        ] {
            let error = read(&members).unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
    }
}
