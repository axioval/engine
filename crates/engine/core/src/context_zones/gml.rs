//! The GML 3 subset the cadastral (#319) and zoning-plan (#321) readers
//! share: the document as a tree of elements by local name, its feature
//! members, the one reference system its geometries state, and plan
//! geometries.
//!
//! Features are the element children of every `featureMember`,
//! `featureMembers` and `member` element, at any depth outside a feature.
//! Geometries read:
//!
//! - areas: `Polygon`, `Surface` of `PolygonPatch` patches, and
//!   `MultiSurface` or `CompositeSurface` of those (`surfaceMember`,
//!   `surfaceMembers`); a ring is a `LinearRing` or a `Ring` of
//!   `curveMember` lines;
//! - lines: `LineString`, `Curve` of `LineStringSegment` segments, and a
//!   `MultiCurve` of one member;
//! - positions as `posList` or `pos`, two or three numbers each
//!   (`srsDimension` on the list or a geometry above it; two without),
//!   of which the plan two are kept, easting first.
//!
//! Anything else (an arc, a circle, a spline, a `coordinates` list, a
//! solid, a point) is refused by name, never approximated.

use std::collections::BTreeMap;

use super::ZoneShape;
use crate::coordinate_system::same_reference_system;
use crate::table_files::{Xml, walk_document};
use crate::zone_area::PlanPolygon;

/// One element: its local name, attributes by local name, element
/// children in order and its own text.
#[derive(Debug, Default)]
pub(super) struct Node {
    pub(super) name: String,
    pub(super) attributes: BTreeMap<String, String>,
    pub(super) children: Vec<Node>,
    text: String,
}

impl Node {
    /// The first child named `name`.
    pub(super) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child named `name`.
    pub(super) fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// Its text, trimmed.
    pub(super) fn text(&self) -> &str {
        self.text.trim()
    }

    /// The attribute `name`.
    pub(super) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

/// The root element of the document `bytes`, `what` naming it in every
/// refusal.
pub(super) fn parse(bytes: &[u8], what: &str) -> Result<Node, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("the {what} is not UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut stack = vec![Node::default()];
    walk_document(text, what, |event| {
        match event {
            Xml::Open(name, attributes) => stack.push(Node {
                name,
                attributes,
                ..Node::default()
            }),
            Xml::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text);
                }
            }
            Xml::Close(_) => {
                let node = stack.pop().filter(|_| !stack.is_empty());
                match (node, stack.last_mut()) {
                    (Some(node), Some(parent)) => parent.children.push(node),
                    _ => return Err(format!("malformed {what} XML: an element closes twice")),
                }
            }
        }
        Ok(())
    })?;
    let mut document = stack
        .pop()
        .filter(|_| stack.is_empty())
        .ok_or_else(|| format!("malformed {what} XML: an element is not closed"))?;
    if document.children.len() != 1 {
        return Err(format!("the {what} has no single root element"));
    }
    Ok(document.children.remove(0))
}

/// Every feature of `root`, in document order: the element children of
/// each feature-member element outside a feature.
pub(super) fn features(root: &Node) -> Vec<&Node> {
    fn walk<'a>(node: &'a Node, found: &mut Vec<&'a Node>) {
        for child in &node.children {
            if matches!(
                child.name.as_str(),
                "featureMember" | "featureMembers" | "member"
            ) {
                found.extend(&child.children);
            } else {
                walk(child, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

/// The reference systems whose coordinates the subset reads, easting
/// first: ETRS89 / UTM zones 28 to 38 (EPSG:25828 to EPSG:25838) under any
/// spelling [`same_reference_system`] matches, and the same systems as the
/// German surveying authorities name them (`urn:adv:crs:ETRS89_UTM28` to
/// `…_UTM38`). A system whose axis order the subset does not know is
/// refused, never read in a guessed order.
pub(super) fn easting_first(name: &str) -> bool {
    (25828..=25838).any(|code| same_reference_system(name, &format!("EPSG:{code}")))
        || name
            .trim()
            .strip_prefix("urn:adv:crs:ETRS89_UTM")
            .and_then(|zone| zone.parse::<u32>().ok())
            .is_some_and(|zone| (28..=38).contains(&zone))
}

/// What the geometries above a position state for it.
#[derive(Clone, Copy, Default)]
struct Inherited<'a> {
    srs: Option<&'a str>,
    dimension: Option<usize>,
}

impl<'a> Inherited<'a> {
    /// What `node` states, over what is inherited.
    fn under(self, node: &'a Node) -> Result<Self, String> {
        let dimension = match node.attribute("srsDimension") {
            Some(text) => Some(
                text.trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|dimension| (2..=3).contains(dimension))
                    .ok_or_else(|| format!("`srsDimension` {text} is not 2 or 3"))?,
            ),
            None => self.dimension,
        };
        Ok(Self {
            srs: node.attribute("srsName").or(self.srs),
            dimension,
        })
    }
}

/// A geometry as read: its shape and the reference system it states.
pub(super) struct Geometry {
    pub(super) shape: ZoneShape,
    pub(super) srs: Option<String>,
}

/// The one geometry the property element `property` (such as `position`)
/// holds, stated in `srs` unless it names its own.
pub(super) fn geometry(property: &Node, srs: Option<&str>) -> Result<Geometry, String> {
    let [geometry] = property.children.as_slice() else {
        return Err(format!(
            "`{}` holds {} geometries, not one",
            property.name,
            property.children.len()
        ));
    };
    let inherited = Inherited {
        srs,
        dimension: None,
    }
    .under(geometry)?;
    let shape = match geometry.name.as_str() {
        "Polygon" | "Surface" | "MultiSurface" | "CompositeSurface" => {
            ZoneShape::Area(surfaces(geometry, inherited)?)
        }
        "LineString" | "Curve" | "MultiCurve" => ZoneShape::Line(line(geometry, inherited)?),
        other => return Err(unread_geometry(other)),
    };
    Ok(Geometry {
        shape,
        srs: inherited.srs.map(str::to_owned),
    })
}

fn unread_geometry(name: &str) -> String {
    format!(
        "a `{name}` geometry is not read; the GML subset reads `Polygon`, `Surface`, \
         `MultiSurface`, `CompositeSurface`, `LineString`, `Curve` and `MultiCurve` of \
         straight segments"
    )
}

/// The polygons of a surface geometry.
fn surfaces(node: &Node, inherited: Inherited<'_>) -> Result<Vec<PlanPolygon>, String> {
    let inherited = inherited.under(node)?;
    match node.name.as_str() {
        "Polygon" | "PolygonPatch" => Ok(vec![polygon(node, inherited)?]),
        "Surface" => {
            let patches = node
                .child("patches")
                .ok_or("a `Surface` states no `patches`")?;
            let mut polygons = Vec::new();
            for patch in &patches.children {
                if patch.name != "PolygonPatch" {
                    return Err(format!(
                        "a `{}` patch is not read; the GML subset reads `PolygonPatch`",
                        patch.name
                    ));
                }
                polygons.extend(surfaces(patch, inherited)?);
            }
            if polygons.is_empty() {
                return Err("a `Surface` states no patch".into());
            }
            Ok(polygons)
        }
        "MultiSurface" | "CompositeSurface" => {
            let mut polygons = Vec::new();
            for member in &node.children {
                if !matches!(member.name.as_str(), "surfaceMember" | "surfaceMembers") {
                    continue;
                }
                for surface in &member.children {
                    polygons.extend(surfaces(surface, inherited)?);
                }
            }
            if polygons.is_empty() {
                return Err(format!("a `{}` states no surface", node.name));
            }
            Ok(polygons)
        }
        other => Err(unread_geometry(other)),
    }
}

/// A polygon of an `exterior` ring and its `interior` rings.
fn polygon(node: &Node, inherited: Inherited<'_>) -> Result<PlanPolygon, String> {
    let ring_of = |boundary: &Node| -> Result<Vec<[f64; 2]>, String> {
        let [ring] = boundary.children.as_slice() else {
            return Err(format!("`{}` holds no single ring", boundary.name));
        };
        ring_points(ring, inherited)
    };
    let outer = ring_of(
        node.child("exterior")
            .ok_or_else(|| format!("a `{}` states no `exterior`", node.name))?,
    )?;
    let holes = node
        .children_named("interior")
        .map(ring_of)
        .collect::<Result<Vec<_>, _>>()?;
    PlanPolygon::try_new(outer, holes).map_err(|_| {
        "a ring has fewer than three points, repeats a point beside itself or encloses no area"
            .to_owned()
    })
}

/// The points of a `LinearRing` or a `Ring` of curve members.
fn ring_points(ring: &Node, inherited: Inherited<'_>) -> Result<Vec<[f64; 2]>, String> {
    let inherited = inherited.under(ring)?;
    match ring.name.as_str() {
        "LinearRing" => positions(ring, inherited),
        "Ring" => {
            let mut points: Vec<[f64; 2]> = Vec::new();
            for member in ring.children_named("curveMember") {
                let [curve] = member.children.as_slice() else {
                    return Err("a `curveMember` holds no single curve".into());
                };
                chain(&mut points, line(curve, inherited)?)?;
            }
            if points.is_empty() {
                return Err("a `Ring` states no `curveMember`".into());
            }
            Ok(points)
        }
        other => Err(format!(
            "a `{other}` ring is not read; the GML subset reads `LinearRing` and `Ring`"
        )),
    }
}

/// Appends `next` to `points`, which it must continue where they end.
fn chain(points: &mut Vec<[f64; 2]>, next: Vec<[f64; 2]>) -> Result<(), String> {
    match (points.last(), next.first()) {
        (None, _) => points.extend(next),
        #[allow(clippy::float_cmp)] // Equal as stated.
        (Some(end), Some(start)) if end == start => points.extend(next.into_iter().skip(1)),
        _ => return Err("a curve does not continue where the one before it ends".into()),
    }
    Ok(())
}

/// The points of a line geometry, in order.
fn line(node: &Node, inherited: Inherited<'_>) -> Result<Vec<[f64; 2]>, String> {
    let inherited = inherited.under(node)?;
    let points = match node.name.as_str() {
        "LineString" | "LineStringSegment" => positions(node, inherited)?,
        "Curve" => {
            let segments = node
                .child("segments")
                .ok_or("a `Curve` states no `segments`")?;
            let mut points = Vec::new();
            for segment in &segments.children {
                if segment.name != "LineStringSegment" {
                    return Err(format!(
                        "a `{}` segment is not read; the GML subset reads straight \
                         `LineStringSegment` segments only, never an arc approximated",
                        segment.name
                    ));
                }
                chain(&mut points, line(segment, inherited)?)?;
            }
            points
        }
        "MultiCurve" => {
            let curves: Vec<&Node> = node
                .children
                .iter()
                .filter(|member| matches!(member.name.as_str(), "curveMember" | "curveMembers"))
                .flat_map(|member| &member.children)
                .collect();
            let [curve] = curves.as_slice() else {
                return Err(format!(
                    "a `MultiCurve` of {} curves is not read; the GML subset reads one",
                    curves.len()
                ));
            };
            line(curve, inherited)?
        }
        other => return Err(unread_geometry(other)),
    };
    #[allow(clippy::float_cmp)] // Equal as stated.
    let repeated = points.windows(2).any(|pair| pair[0] == pair[1]);
    if points.len() < 2 || repeated {
        return Err("a line has fewer than two points or repeats a point beside itself".into());
    }
    Ok(points)
}

/// The plan positions of a `posList`, or of `pos` children, of `node`.
fn positions(node: &Node, inherited: Inherited<'_>) -> Result<Vec<[f64; 2]>, String> {
    if node.child("coordinates").is_some() {
        return Err(
            "a `coordinates` list is not read; the GML subset reads `posList` and `pos`".into(),
        );
    }
    let numbers = |text: &str| -> Result<Vec<f64>, String> {
        text.split_whitespace()
            .map(|number| {
                number
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("`{number}` is not a finite coordinate"))
            })
            .collect()
    };
    if let Some(list) = node.child("posList") {
        let inherited = inherited.under(list)?;
        let dimension = inherited.dimension.unwrap_or(2);
        let values = numbers(list.text())?;
        if values.len() % dimension != 0 {
            return Err(format!(
                "a `posList` of {} numbers is no list of {dimension}-dimensional positions",
                values.len()
            ));
        }
        return Ok(values
            .chunks(dimension)
            .map(|chunk| [chunk[0], chunk[1]])
            .collect());
    }
    let points = node
        .children_named("pos")
        .map(|pos| {
            let values = numbers(pos.text())?;
            match values.as_slice() {
                [x, y] | [x, y, _] => Ok([*x, *y]),
                _ => Err(format!(
                    "a `pos` of {} numbers is no position",
                    values.len()
                )),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    if points.is_empty() {
        return Err(format!("a `{}` states no `posList` or `pos`", node.name));
    }
    Ok(points)
}

/// The one reference system `names` (each feature's, in order) state,
/// else `fallback` (the document's envelope), checked to be read easting
/// first.
pub(super) fn reference(
    names: impl IntoIterator<Item = Option<String>>,
    fallback: Option<&str>,
) -> Result<super::ZoneReference, String> {
    let mut stated: Option<String> = None;
    for name in names {
        let Some(name) = name.or_else(|| fallback.map(str::to_owned)) else {
            return Err(
                "a geometry states no reference system (`srsName` on it, above it or on the \
                 document's envelope)"
                    .into(),
            );
        };
        match &stated {
            Some(first) if !same_reference_system(first, &name) => {
                return Err(format!(
                    "the geometries state two reference systems, `{first}` and `{name}`"
                ));
            }
            Some(_) => {}
            None => stated = Some(name),
        }
    }
    let name = stated.ok_or("it states no feature the reader reads")?;
    if name.trim() == super::MODEL_REFERENCE {
        return Ok(super::ZoneReference::Model);
    }
    if !easting_first(&name) {
        return Err(format!(
            "the reference system `{name}` is not read: the GML subset reads ETRS89 / UTM \
             (EPSG:25828 to EPSG:25838, or `urn:adv:crs:ETRS89_UTM…`), easting first, or \
             `{}`, and never guesses another system's axis order",
            super::MODEL_REFERENCE
        ));
    }
    Ok(super::ZoneReference::Map(name.trim().to_owned()))
}

/// The reference system the first envelope of the document or its feature
/// collections (`boundedBy/Envelope`, outside every feature) names, if any.
pub(super) fn envelope_reference(root: &Node) -> Option<&str> {
    if let Some(name) = root.child("boundedBy").and_then(|bounds| {
        bounds
            .children
            .iter()
            .find_map(|envelope| envelope.attribute("srsName"))
    }) {
        return Some(name);
    }
    root.children
        .iter()
        .filter(|child| {
            !matches!(
                child.name.as_str(),
                "featureMember" | "featureMembers" | "member"
            )
        })
        .find_map(envelope_reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(xml: &str) -> Result<Geometry, String> {
        let root = parse(format!("<position>{xml}</position>").as_bytes(), "test")?;
        geometry(&root, Some("EPSG:25832"))
    }

    #[test]
    fn surfaces_rings_and_lines_of_straight_segments_are_read() {
        let square = "0 0 10 0 10 10 0 10 0 0";
        let polygon = shape(&format!(
            "<gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>{square}</gml:posList>\
             </gml:LinearRing></gml:exterior></gml:Polygon>"
        ))
        .unwrap();
        assert!(matches!(&polygon.shape, ZoneShape::Area(p) if p.len() == 1));
        let patch = shape(
            "<Surface srsName=\"urn:adv:crs:ETRS89_UTM32\"><patches><PolygonPatch><exterior><Ring>\
             <curveMember><Curve><segments><LineStringSegment><posList srsDimension=\"3\">\
             0 0 1 10 0 1 10 10 1</posList></LineStringSegment><LineStringSegment>\
             <pos>10 10</pos><pos>0 10</pos><pos>0 0</pos></LineStringSegment></segments>\
             </Curve></curveMember></Ring></exterior></PolygonPatch></patches></Surface>",
        )
        .unwrap();
        assert_eq!(patch.srs.as_deref(), Some("urn:adv:crs:ETRS89_UTM32"));
        let ZoneShape::Area(polygons) = &patch.shape else {
            panic!("an area");
        };
        assert_eq!(polygons[0].outer().len(), 4);
        let line = shape("<LineString><posList>0 5 30 5</posList></LineString>").unwrap();
        assert_eq!(line.shape, ZoneShape::Line(vec![[0.0, 5.0], [30.0, 5.0]]));
    }

    #[test]
    fn what_the_subset_does_not_read_is_refused_by_name() {
        for (xml, why) in [
            ("<Point><pos>0 0</pos></Point>", "`Point` geometry"),
            (
                "<Curve><segments><Arc><posList>0 0 1 1 2 0</posList></Arc></segments></Curve>",
                "`Arc` segment",
            ),
            (
                "<LineString><coordinates>0,0 1,1</coordinates></LineString>",
                "`coordinates`",
            ),
            (
                "<LineString><posList srsDimension=\"3\">0 0 0 1</posList></LineString>",
                "3-dimensional",
            ),
            (
                "<MultiCurve><curveMember><LineString><posList>0 0 1 1</posList></LineString>\
                 </curveMember><curveMember><LineString><posList>2 2 3 3</posList>\
                 </LineString></curveMember></MultiCurve>",
                "2 curves",
            ),
        ] {
            let error = shape(xml).err().unwrap();
            assert!(error.contains(why), "{why}: {error}");
        }
    }

    #[test]
    fn only_reference_systems_read_easting_first_are_read() {
        assert!(easting_first("EPSG:25832"));
        assert!(easting_first("urn:ogc:def:crs:EPSG::25833"));
        assert!(easting_first("urn:adv:crs:ETRS89_UTM32"));
        assert!(!easting_first("EPSG:31467"));
        assert!(!easting_first("EPSG:4326"));
        assert!(
            reference([Some("EPSG:31467".into())], None)
                .unwrap_err()
                .contains("axis order")
        );
        assert!(
            reference([Some("EPSG:25832".into()), Some("EPSG:25833".into())], None)
                .unwrap_err()
                .contains("two reference systems")
        );
        assert!(reference([None], None).unwrap_err().contains("srsName"));
        assert_eq!(
            reference([None], Some("EPSG:25832")).unwrap(),
            super::super::ZoneReference::Map("EPSG:25832".into())
        );
    }
}
