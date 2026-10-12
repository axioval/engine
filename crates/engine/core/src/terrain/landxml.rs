//! `LandXML` 1.x surfaces: the triangulated (`surfType="TIN"`) definition
//! of one `Surface`, its points (`Pnts/P`, each `northing easting
//! elevation`) and faces (`Faces/F`, three point ids; an invisible face,
//! `i="1"`, is left out), in the linear unit `Units` states and the
//! coordinate system `CoordinateSystem` names.
//!
//! `LandXML` states no vertical accuracy for a surface, so the reader takes
//! it from its caller; a unit it does not know, a file stating none, a
//! grid definition and a face naming a point the surface lacks are
//! refused, never guessed.

use std::collections::HashMap;

use super::{TerrainError, TerrainFrame, TerrainKind, TerrainSurface};
use crate::table_files::{Xml, sha256_hex, walk_document};

/// Metres per `LandXML` linear unit, by its enumeration name.
const LINEAR_UNITS: &[(&str, f64)] = &[
    ("meter", 1.0),
    ("kilometer", 1000.0),
    ("decimeter", 0.1),
    ("centimeter", 0.01),
    ("millimeter", 0.001),
    ("foot", 0.3048),
    ("USSurveyFoot", 1200.0 / 3937.0),
    ("inch", 0.0254),
];

/// One `Surface` element as read.
#[derive(Default)]
struct Read {
    name: String,
    surface_type: Option<String>,
    points: Vec<[f64; 3]>,
    ids: HashMap<String, u32>,
    faces: Vec<[String; 3]>,
}

/// What a document states, as its elements are walked.
#[derive(Default)]
struct Document {
    stack: Vec<String>,
    unit: Option<f64>,
    crs: Option<String>,
    surfaces: Vec<Read>,
    /// The point or face whose text is being collected: its element, a
    /// point's id and whether a face is invisible.
    open: Option<(String, Option<String>, bool)>,
    text: String,
}

type Attributes = std::collections::BTreeMap<String, String>;

impl Document {
    /// Whether the innermost open elements are `path`, outermost first.
    fn inside(&self, path: &[&str]) -> bool {
        self.stack.len() >= path.len()
            && self.stack[self.stack.len() - path.len()..]
                .iter()
                .zip(path)
                .all(|(held, wanted)| held == wanted)
    }

    fn on(&mut self, event: Xml) -> Result<(), String> {
        match event {
            Xml::Open(name, attributes) => {
                self.open_element(&name, &attributes)?;
                self.stack.push(name);
            }
            Xml::Text(text) => {
                if self.open.is_some() {
                    self.text.push_str(&text);
                }
            }
            Xml::Close(name) => {
                self.stack.pop();
                if let Some((element, id, invisible)) =
                    self.open.take_if(|(held, ..)| *held == name)
                {
                    self.close_element(&element, id, invisible)?;
                }
            }
        }
        Ok(())
    }

    fn open_element(&mut self, name: &str, attributes: &Attributes) -> Result<(), String> {
        match name {
            "Metric" | "Imperial" if self.inside(&["Units"]) => {
                if let Some(linear) = attributes.get("linearUnit") {
                    let metres = LINEAR_UNITS
                        .iter()
                        .find(|(known, _)| known == linear)
                        .map(|(_, metres)| *metres)
                        .ok_or_else(|| format!("the linear unit `{linear}` is not supported"))?;
                    self.unit = Some(metres);
                }
            }
            "CoordinateSystem" => {
                let stated = |key: &str| {
                    attributes
                        .get(key)
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                };
                self.crs = stated("epsgCode")
                    .map(|code| format!("EPSG:{code}"))
                    .or_else(|| stated("name").map(str::to_owned));
            }
            "Surface" if self.inside(&["Surfaces"]) => self.surfaces.push(Read {
                name: attributes.get("name").cloned().unwrap_or_default(),
                ..Read::default()
            }),
            "Definition" if self.inside(&["Surface"]) => {
                if let Some(read) = self.surfaces.last_mut() {
                    read.surface_type = attributes.get("surfType").cloned();
                }
            }
            "P" if self.inside(&["Definition", "Pnts"]) => {
                self.open = Some((name.to_owned(), attributes.get("id").cloned(), false));
                self.text.clear();
            }
            "F" if self.inside(&["Definition", "Faces"]) => {
                let invisible = attributes.get("i").is_some_and(|i| i.trim() == "1");
                self.open = Some((name.to_owned(), None, invisible));
                self.text.clear();
            }
            _ => {}
        }
        Ok(())
    }

    fn close_element(
        &mut self,
        element: &str,
        id: Option<String>,
        invisible: bool,
    ) -> Result<(), String> {
        let read = self
            .surfaces
            .last_mut()
            .ok_or_else(|| "a point or face lies outside any surface".to_owned())?;
        let values: Vec<&str> = self.text.split_whitespace().collect();
        if element == "P" {
            let id =
                id.ok_or_else(|| format!("surface `{}`: a point states no `id`", read.name))?;
            let numbers: Option<Vec<f64>> = values.iter().map(|value| value.parse().ok()).collect();
            let Some([northing, easting, elevation]) = numbers
                .as_deref()
                .and_then(|numbers| <[f64; 3]>::try_from(numbers).ok())
            else {
                return Err(format!(
                    "surface `{}`: point `{id}` is not three numbers (northing, easting, \
                     elevation)",
                    read.name
                ));
            };
            let index = u32::try_from(read.points.len())
                .map_err(|_| "the surface has too many points".to_owned())?;
            if read.ids.insert(id.clone(), index).is_some() {
                return Err(format!(
                    "surface `{}`: point `{id}` is stated twice",
                    read.name
                ));
            }
            read.points.push([easting, northing, elevation]);
        } else if !invisible {
            let [a, b, c] = values[..] else {
                return Err(format!(
                    "surface `{}`: a face does not name three points",
                    read.name
                ));
            };
            read.faces.push([a.to_owned(), b.to_owned(), c.to_owned()]);
        }
        Ok(())
    }

    /// The surface named `wanted`, or the only one.
    fn chosen(mut self, wanted: Option<&str>) -> Result<Read, TerrainError> {
        let names = self
            .surfaces
            .iter()
            .map(|read| format!("`{}`", read.name))
            .collect::<Vec<_>>()
            .join(", ");
        let index = match wanted {
            Some(wanted) => self
                .surfaces
                .iter()
                .position(|read| read.name == wanted)
                .ok_or_else(|| {
                    TerrainError::Unreadable(format!(
                        "the file has no surface `{wanted}`; it has {}",
                        if names.is_empty() { "none" } else { &names }
                    ))
                })?,
            None => match self.surfaces.len() {
                0 => return Err(TerrainError::Unreadable("the file has no surface".into())),
                1 => 0,
                _ => {
                    return Err(TerrainError::Unreadable(format!(
                        "the file has several surfaces ({names}); name one as FILE#SURFACE"
                    )));
                }
            },
        };
        let read = self.surfaces.swap_remove(index);
        match read.surface_type.as_deref() {
            Some(kind) if kind.eq_ignore_ascii_case("TIN") => Ok(read),
            Some(other) => Err(TerrainError::Unreadable(format!(
                "surface `{}` is a `{other}` surface, not a TIN",
                read.name
            ))),
            None => Err(TerrainError::Unreadable(format!(
                "surface `{}` states no definition type",
                read.name
            ))),
        }
    }
}

impl Read {
    /// The faces as indices into the points.
    fn triangles(&self) -> Result<Vec<[u32; 3]>, TerrainError> {
        self.faces
            .iter()
            .map(|face| {
                let mut corners = [0u32; 3];
                for (slot, id) in corners.iter_mut().zip(face) {
                    *slot = *self.ids.get(id).ok_or_else(|| {
                        TerrainError::Unreadable(format!(
                            "surface `{}`: a face names point `{id}`, which it does not state",
                            self.name
                        ))
                    })?;
                }
                Ok(corners)
            })
            .collect()
    }
}

impl TerrainSurface {
    /// The surface named `surface` (or the file's only one) of the
    /// `LandXML` document `bytes`, read from the file `origin`, as a terrain
    /// of `kind` in `frame` with the stated vertical accuracy.
    ///
    /// A surface in model coordinates names no coordinate system; the
    /// document's `CoordinateSystem` (`epsgCode`, else `name`) is kept for
    /// a map surface only.
    pub fn read_landxml(
        bytes: &[u8],
        origin: &str,
        surface: Option<&str>,
        kind: TerrainKind,
        accuracy_metres: f64,
        frame: TerrainFrame,
    ) -> Result<Self, TerrainError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| TerrainError::Unreadable("the file is not UTF-8".into()))?;
        let mut document = Document::default();
        walk_document(text, "LandXML", |event| document.on(event))
            .map_err(TerrainError::Unreadable)?;
        let unit = document.unit.ok_or_else(|| {
            TerrainError::Unreadable("the file states no linear unit (`Units`)".into())
        })?;
        let crs = match frame {
            TerrainFrame::Map => document.crs.take(),
            TerrainFrame::Model => None,
        };
        let read = document.chosen(surface)?;
        let triangles = read.triangles()?;
        let points: Vec<[f64; 3]> = read
            .points
            .iter()
            .map(|point| point.map(|value| value * unit))
            .collect();
        Self::try_new(
            kind,
            read.name,
            frame,
            crs,
            accuracy_metres,
            (origin, sha256_hex(bytes)),
            &points,
            triangles,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{TerrainPointRequest, TerrainService, TinTerrain};

    /// A square of 100 m in millimetres, rising 0.1 m per metre eastwards,
    /// with an invisible face over it that must be left out.
    const SQUARE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <Units><Metric linearUnit="millimeter" areaUnit="squareMeter" volumeUnit="cubicMeter"/></Units>
  <CoordinateSystem epsgCode="25832" name="ETRS89 / UTM zone 32N"/>
  <Surfaces>
    <Surface name="Ground">
      <Definition surfType="TIN">
        <Pnts>
          <P id="a">0 0 100000</P>
          <P id="b">0 100000 110000</P>
          <P id="c">100000 100000 110000</P>
          <P id="d">100000 0 100000</P>
          <P id="e">50000 50000 900000</P>
        </Pnts>
        <Faces>
          <F>a b c</F>
          <F>a c d</F>
          <F i="1">a b e</F>
        </Faces>
      </Definition>
    </Surface>
  </Surfaces>
</LandXML>"#;

    fn read(
        text: &str,
        surface: Option<&str>,
        frame: TerrainFrame,
    ) -> Result<TerrainSurface, TerrainError> {
        TerrainSurface::read_landxml(
            text.as_bytes(),
            "ground.xml",
            surface,
            TerrainKind::Original,
            0.05,
            frame,
        )
    }

    #[test]
    fn a_synthetic_tin_is_read_in_metres_easting_first() {
        let surface = read(SQUARE, None, TerrainFrame::Map).unwrap();
        assert_eq!(surface.name(), "Ground");
        assert_eq!(surface.point_count(), 5);
        assert_eq!(
            surface.triangle_count(),
            2,
            "the invisible face is left out"
        );
        assert_eq!(surface.crs(), Some("EPSG:25832"));
        assert_eq!(surface.sha256(), sha256_hex(SQUARE.as_bytes()));
        let terrain = TinTerrain::try_new(vec![surface]).unwrap();
        // Easting 80 m, northing 10 m: 100 m + 0.1 * 80.
        let request = TerrainPointRequest::try_new(TerrainKind::Original, [80.0, 10.0]).unwrap();
        let elevation = terrain.measure_elevation(&request).unwrap();
        assert!(
            (elevation.lower_metres() - 107.95).abs() < 1e-6,
            "{elevation:?}"
        );
        assert!(
            (elevation.upper_metres() - 108.05).abs() < 1e-6,
            "{elevation:?}"
        );
        assert!((elevation.accuracy_metres() - 0.05).abs() < f64::EPSILON);
        // In model coordinates the map's name is not kept.
        assert_eq!(read(SQUARE, None, TerrainFrame::Model).unwrap().crs(), None);
    }

    #[test]
    fn what_cannot_be_read_exactly_is_refused() {
        let cases = [
            (SQUARE.replace("millimeter", "furlong"), "linear unit `furlong`"),
            (
                SQUARE.replace(
                    "<Units><Metric linearUnit=\"millimeter\" areaUnit=\"squareMeter\" volumeUnit=\"cubicMeter\"/></Units>",
                    "",
                ),
                "states no linear unit",
            ),
            (SQUARE.replace("surfType=\"TIN\"", "surfType=\"grid\""), "not a TIN"),
            (SQUARE.replace("<F>a c d</F>", "<F>a c z</F>"), "point `z`"),
            (SQUARE.replace("0 0 100000", "0 0"), "three numbers"),
            (SQUARE.replace("<P id=\"b\">", "<P id=\"a\">"), "stated twice"),
            (SQUARE.replace("<Surfaces>", "<!DOCTYPE x><Surfaces>"), "LandXML XML declares a document type"),
        ];
        for (text, expected) in cases {
            let refused = read(&text, None, TerrainFrame::Map)
                .unwrap_err()
                .to_string();
            assert!(refused.contains(expected), "{refused} lacks {expected}");
        }
        let refused = read(SQUARE, Some("Design"), TerrainFrame::Map)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("no surface `Design`; it has `Ground`"),
            "{refused}"
        );
        let two = SQUARE.replace(
            "</Surfaces>",
            "<Surface name=\"Design\"><Definition surfType=\"TIN\"><Pnts/><Faces/></Definition></Surface></Surfaces>",
        );
        let refused = read(&two, None, TerrainFrame::Map).unwrap_err().to_string();
        assert!(
            refused.contains("several surfaces (`Ground`, `Design`)"),
            "{refused}"
        );
        assert_eq!(
            read(&two, Some("Ground"), TerrainFrame::Map)
                .unwrap()
                .name(),
            "Ground"
        );
    }
}
