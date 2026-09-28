//! How an object's body is modelled, read as properties in [`BODY_SET`](axioval_ir::BODY_SET).
//!
//! The body is the product's `Body` representation (`ifc-geometry`'s
//! `select_shape_representation`), described by `ifc-geometry`'s
//! `body_description` (openbimrs/ifc#147): one entry per geometric item,
//! mapped items resolved, each with its kind and, for a swept-area solid,
//! its profile parameters, placement and path in world coordinates. Nothing
//! here re-reads representation or profile slots; the description is the
//! one the geometry lowering itself is built from, so the facts and the
//! meshed body cannot disagree.
//!
//! Units come from the project's exact units (`ifc_properties::exact_unit`),
//! never from `ifc_geometry::units::resolve`, which assumes metres and
//! radians when a file states none. An unresolvable length unit refuses the
//! whole body; an unresolvable plane-angle unit refuses the angles alone.
//!
//! The description is all or nothing: an item it cannot read exactly refuses
//! the whole body rather than answering for part of it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use axioval_engine::PropertyResolutionError;
use axioval_ir::{PropertyValue, QuantityDimension};
use ifc_geometry::{
    BodyDescription, BodyItem, BodyKind, GeometryError, ProfileDescription, ProfileParameters,
    SweepPath, SweptSolid, Transform, UnitScale, body_description, profile_outline,
};
use ifc_model::{EntityId, Model};
use ifc_properties::exact_unit;

use crate::attributes::AttributeValue;
use crate::release::Release;

/// One fact: its value and the locator detail that proves it, or why it is
/// refused.
type Fact = Result<(PropertyValue, String), PropertyResolutionError>;

/// Every fact of one object's body, keyed by the lower-case property name.
struct Facts {
    facts: BTreeMap<String, Fact>,
    /// Number of items, to tell an ambiguous unprefixed name from an absent one.
    items: usize,
}

/// One object's body facts, `None` without a body, or why they are refused.
type Read = Result<Option<Facts>, PropertyResolutionError>;

/// Answers [`BODY_SET`](axioval_ir::BODY_SET) requests for one model.
pub(crate) struct Bodies {
    release: Release,
    units: OnceLock<Result<Units, String>>,
    cache: Mutex<BTreeMap<EntityId, Arc<Read>>>,
}

/// The project's units as exactly as they resolve.
#[derive(Clone)]
struct Units {
    scale: UnitScale,
    /// Why plane angles cannot be converted, when they cannot.
    angle: Option<String>,
}

impl Bodies {
    pub(crate) fn new(release: Release) -> Self {
        Self {
            release,
            units: OnceLock::new(),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Reads `name` in [`BODY_SET`](axioval_ir::BODY_SET) for `object`; `Ok(None)` is exact absence.
    pub(crate) fn resolve(
        &self,
        model: &Model,
        object: EntityId,
        name: &str,
    ) -> Result<Option<AttributeValue>, PropertyResolutionError> {
        let facts = self.facts(model, object)?;
        let Some(facts) = facts.as_ref().as_ref().map_err(Clone::clone)? else {
            return Ok(None);
        };
        let key = name.to_ascii_lowercase();
        if let Some(fact) = facts.facts.get(&key) {
            let (value, detail) = fact.clone()?;
            return Ok(Some(AttributeValue { value, detail }));
        }
        // An unprefixed item name over several items names no one item.
        if facts.items > 1 && facts.facts.contains_key(&format!("item1.{key}")) {
            return Err(PropertyResolutionError::Conflicting(format!(
                "the body of {object} has {} items; read `Item<n>.{name}` for one of them",
                facts.items
            )));
        }
        Ok(None)
    }

    fn facts(&self, model: &Model, object: EntityId) -> Result<Arc<Read>, PropertyResolutionError> {
        let mut cache = self.cache.lock().map_err(|_| {
            PropertyResolutionError::Unavailable("body cache lock is poisoned".into())
        })?;
        if let Some(known) = cache.get(&object) {
            return Ok(known.clone());
        }
        let facts = Arc::new(self.read(model, object));
        cache.insert(object, facts.clone());
        Ok(facts)
    }

    fn read(
        &self,
        model: &Model,
        object: EntityId,
    ) -> Result<Option<Facts>, PropertyResolutionError> {
        let entity = model
            .get(object)
            .ok_or(PropertyResolutionError::InvalidRequest)?;
        if !self.release.schema.is_a(&entity.type_name, "IFCPRODUCT") {
            // Only products carry a shape.
            return Ok(None);
        }
        let units = self
            .units
            .get_or_init(|| units(model))
            .clone()
            .map_err(PropertyResolutionError::Incomplete)?;
        let Some(body) = body_description(model, &units.scale, object).map_err(geometry_error)?
        else {
            return Ok(None);
        };
        Ok(Some(Facts::of(model, object, &body, &units)))
    }
}

/// The project length unit, which must resolve, and the plane-angle unit,
/// which refuses only the angles when it does not.
fn units(model: &Model) -> Result<Units, String> {
    let scale = |measure: &str| match exact_unit(model, measure, None) {
        Ok(unit) if unit.offset == 0.0 && unit.scale.is_finite() && unit.scale > 0.0 => {
            Ok(unit.scale)
        }
        Ok(_) => Err(format!(
            "the project {measure} unit has no positive finite scale"
        )),
        Err(error) => Err(format!(
            "the project {measure} unit cannot be resolved exactly: {error}"
        )),
    };
    let length = scale("IFCLENGTHMEASURE")?;
    let (angle_scale, angle) = match scale("IFCPLANEANGLEMEASURE") {
        Ok(value) => (value, None),
        // NaN marks every angle read under it, and each is refused alone.
        Err(message) => (f64::NAN, Some(message)),
    };
    Ok(Units {
        scale: UnitScale {
            length_to_metres: length,
            angle_to_radians: angle_scale,
        },
        angle,
    })
}

fn geometry_error(error: GeometryError) -> PropertyResolutionError {
    match error {
        GeometryError::Unsupported { .. } => {
            PropertyResolutionError::Unavailable(format!("the body cannot be described: {error}"))
        }
        other => {
            PropertyResolutionError::Incomplete(format!("the body cannot be described: {other}"))
        }
    }
}

/// The neutral name of an item kind; `None` for a kind this adapter does not
/// know yet, which refuses the item.
fn kind_name(kind: BodyKind) -> Option<&'static str> {
    Some(match kind {
        BodyKind::Extrusion => "extrusion",
        BodyKind::TaperedExtrusion => "tapered-extrusion",
        BodyKind::Revolution => "revolution",
        BodyKind::TaperedRevolution => "tapered-revolution",
        BodyKind::DirectrixSweep => "directrix-sweep",
        BodyKind::SweptDisk => "swept-disk",
        BodyKind::SectionedSpine => "sectioned-spine",
        BodyKind::Brep => "brep",
        BodyKind::Csg => "csg",
        BodyKind::CsgPrimitive => "csg-primitive",
        BodyKind::HalfSpace => "half-space",
        BodyKind::BoundingBox => "bounding-box",
        BodyKind::Tessellated => "tessellation",
        BodyKind::SurfaceModel => "surface-model",
        BodyKind::Face => "face",
        BodyKind::GeometricSet => "geometric-set",
        BodyKind::Curve => "curve",
        BodyKind::Surface => "surface",
        BodyKind::Point => "point",
        _ => return None,
    })
}

/// Collects facts under one prefix and locator.
struct Writer<'a> {
    model: &'a Model,
    facts: &'a mut BTreeMap<String, Fact>,
    units: &'a Units,
    prefix: String,
    detail: String,
}

impl Writer<'_> {
    fn nested(&mut self, prefix: &str, detail: String) -> Writer<'_> {
        Writer {
            model: self.model,
            facts: self.facts,
            units: self.units,
            prefix: format!("{}{prefix}", self.prefix),
            detail,
        }
    }

    fn put(&mut self, name: &str, fact: Result<PropertyValue, PropertyResolutionError>) {
        let key = format!("{}{name}", self.prefix).to_ascii_lowercase();
        let detail = self.detail.clone();
        self.facts.insert(key, fact.map(|value| (value, detail)));
    }

    fn text(&mut self, name: &str, value: &str) {
        self.put(name, Ok(PropertyValue::String(value.to_owned())));
    }

    fn integer(&mut self, name: &str, value: usize) {
        self.put(
            name,
            i64::try_from(value)
                .map(PropertyValue::Integer)
                .map_err(|_| PropertyResolutionError::InvalidValue),
        );
    }

    fn decimal(&mut self, name: &str, value: f64) {
        self.put(
            name,
            if value.is_finite() {
                Ok(PropertyValue::Decimal(value))
            } else {
                Err(PropertyResolutionError::InvalidValue)
            },
        );
    }

    fn length(&mut self, name: &str, value: f64) {
        self.put(name, quantity(value, QuantityDimension::Length));
    }

    fn angle(&mut self, name: &str, value: f64) {
        let fact = match &self.units.angle {
            Some(reason) => Err(PropertyResolutionError::Incomplete(reason.clone())),
            None => quantity(value, QuantityDimension::PlaneAngle),
        };
        self.put(name, fact);
    }

    fn optional_length(&mut self, name: &str, value: Option<f64>) {
        if let Some(value) = value {
            self.length(name, value);
        }
    }

    fn optional_angle(&mut self, name: &str, value: Option<f64>) {
        if let Some(value) = value {
            self.angle(name, value);
        }
    }

    fn vector(&mut self, name: &str, value: [f64; 3]) {
        for (axis, component) in ["X", "Y", "Z"].iter().zip(value) {
            self.decimal(&format!("{name}{axis}"), component);
        }
    }

    fn point(&mut self, name: &str, value: [f64; 3]) {
        for (axis, component) in ["X", "Y", "Z"].iter().zip(value) {
            self.length(&format!("{name}{axis}"), component);
        }
    }
}

fn quantity(
    value: f64,
    dimension: QuantityDimension,
) -> Result<PropertyValue, PropertyResolutionError> {
    if value.is_finite() {
        Ok(PropertyValue::Quantity { value, dimension })
    } else {
        Err(PropertyResolutionError::InvalidValue)
    }
}

impl Facts {
    fn of(model: &Model, object: EntityId, body: &BodyDescription, units: &Units) -> Self {
        let mut facts = BTreeMap::new();
        let detail = format!("body:{object}:{}", body.representation);
        let mut root = Writer {
            model,
            facts: &mut facts,
            units,
            prefix: String::new(),
            detail: detail.clone(),
        };
        root.integer("Count", body.items.len());
        root.put(
            "Mapped",
            Ok(PropertyValue::Boolean(
                body.items.iter().any(|item| !item.mapped_by.is_empty()),
            )),
        );
        root.put("Mirrored", mirrored(body));
        let mut kinds: Vec<&str> = Vec::new();
        let mut unknown = None;
        for item in &body.items {
            match kind_name(item.kind) {
                Some(kind) => kinds.push(kind),
                None => unknown = Some(item.type_name.clone()),
            }
        }
        if !body.items.is_empty() {
            let fact = if let Some(type_name) = unknown {
                Err(PropertyResolutionError::Unavailable(format!(
                    "{type_name} is an item kind this adapter does not name yet"
                )))
            } else {
                kinds.sort_unstable();
                kinds.dedup();
                Ok(PropertyValue::List(
                    kinds
                        .into_iter()
                        .map(|kind| PropertyValue::String(kind.to_owned()))
                        .collect(),
                ))
            };
            root.put("Kinds", fact);
        }
        for (index, item) in body.items.iter().enumerate() {
            let via = item
                .mapped_by
                .iter()
                .fold(String::new(), |via, mapped| format!("{via}{mapped}>"));
            let item_detail = format!("{detail}:{via}{}", item.item);
            let numbered = format!("Item{}.", index + 1);
            write_item(&mut root.nested(&numbered, item_detail.clone()), item);
            if body.items.len() == 1 {
                write_item(&mut root.nested("", item_detail), item);
            }
        }
        Self {
            facts,
            items: body.items.len(),
        }
    }
}

fn write_item(writer: &mut Writer<'_>, item: &BodyItem) {
    match kind_name(item.kind) {
        Some(kind) => writer.text("Kind", kind),
        None => writer.put(
            "Kind",
            Err(PropertyResolutionError::Unavailable(format!(
                "{} is an item kind this adapter does not name yet",
                item.type_name
            ))),
        ),
    }
    writer.put(
        "Mapped",
        Ok(PropertyValue::Boolean(!item.mapped_by.is_empty())),
    );
    if let Some(swept) = &item.swept {
        write_swept(writer, swept);
    }
}

fn write_swept(writer: &mut Writer<'_>, swept: &SweptSolid) {
    let detail = writer.detail.clone();
    write_profile(
        &mut writer.nested(
            "Profile.",
            format!("{detail}:profile:{}", swept.profile.entity),
        ),
        &swept.profile,
    );
    if let Some(end) = &swept.end_profile {
        write_profile(
            &mut writer.nested("EndProfile.", format!("{detail}:profile:{}", end.entity)),
            end,
        );
    }
    write_placement(writer, &swept.placement_world);
    match &swept.path {
        SweepPath::Extrusion {
            direction_world,
            depth,
            ..
        } => {
            writer.length("Extrusion.Depth", *depth);
            writer.vector("Extrusion.Direction", *direction_world);
            // The angle between the extrusion's line (either sense) and the
            // vertical: 0 for a wall extruded up or down, π/2 for a beam.
            let inclination = direction_world[2].abs().min(1.0).acos();
            writer.put(
                "Extrusion.Inclination",
                quantity(inclination, QuantityDimension::PlaneAngle),
            );
        }
        SweepPath::Revolution {
            axis_origin_world,
            axis_direction_world,
            angle,
            ..
        } => {
            writer.angle("Revolution.Angle", *angle);
            writer.point("Revolution.Origin", *axis_origin_world);
            writer.vector("Revolution.Axis", *axis_direction_world);
        }
        // A directrix is a curve; the body states no scalar path facts for it.
        _ => {}
    }
}

/// Whether the transform placing the body reverses its orientation.
///
/// Placements are right-handed by construction, and so is the mapping of a
/// swept solid (`ifc-geometry` refuses one that scales or mirrors it), so a
/// body of such items is not mirrored. The mapping transform of any other
/// mapped item is not described, so its mirroring is refused, never assumed.
fn mirrored(body: &BodyDescription) -> Result<PropertyValue, PropertyResolutionError> {
    if body
        .items
        .iter()
        .all(|item| item.mapped_by.is_empty() || item.swept.is_some())
    {
        Ok(PropertyValue::Boolean(false))
    } else {
        Err(PropertyResolutionError::Unavailable(
            "the mapping transform of a mapped item that is not a swept solid is not described, \
             so whether it mirrors the body cannot be read"
                .into(),
        ))
    }
}

fn write_placement(writer: &mut Writer<'_>, placement: &Transform) {
    writer.point("Placement.Origin", placement.origin);
    let [x, y, z] = placement.basis;
    writer.vector("Placement.XAxis", x);
    writer.vector("Placement.YAxis", y);
    writer.vector("Placement.ZAxis", z);
}

/// The neutral family name and the family's parameters.
#[allow(clippy::too_many_lines)]
fn write_profile(writer: &mut Writer<'_>, profile: &ProfileDescription) {
    if let Some(name) = &profile.name {
        writer.text("Name", name);
    }
    if let Some(position) = &profile.position {
        writer.length("PositionX", position.origin[0]);
        writer.length("PositionY", position.origin[1]);
        // Dimensionless axes: the angle needs no unit.
        writer.put(
            "PositionAngle",
            quantity(
                position.x_axis[1].atan2(position.x_axis[0]),
                QuantityDimension::PlaneAngle,
            ),
        );
    }
    let family = match &profile.parameters {
        ProfileParameters::Rectangle { x_dim, y_dim, .. } => {
            writer.length("XDim", *x_dim);
            writer.length("YDim", *y_dim);
            "rectangle"
        }
        ProfileParameters::RoundedRectangle {
            x_dim,
            y_dim,
            rounding_radius,
            ..
        } => {
            writer.length("XDim", *x_dim);
            writer.length("YDim", *y_dim);
            writer.length("RoundingRadius", *rounding_radius);
            "rounded-rectangle"
        }
        ProfileParameters::RectangleHollow {
            x_dim,
            y_dim,
            wall_thickness,
            inner_fillet_radius,
            outer_fillet_radius,
            ..
        } => {
            writer.length("XDim", *x_dim);
            writer.length("YDim", *y_dim);
            writer.length("WallThickness", *wall_thickness);
            writer.optional_length("InnerFilletRadius", *inner_fillet_radius);
            writer.optional_length("OuterFilletRadius", *outer_fillet_radius);
            "rectangle-hollow"
        }
        ProfileParameters::Circle { radius, .. } => {
            writer.length("Radius", *radius);
            "circle"
        }
        ProfileParameters::CircleHollow {
            radius,
            wall_thickness,
            ..
        } => {
            writer.length("Radius", *radius);
            writer.length("WallThickness", *wall_thickness);
            "circle-hollow"
        }
        ProfileParameters::Ellipse {
            semi_axis_1,
            semi_axis_2,
            ..
        } => {
            writer.length("SemiAxis1", *semi_axis_1);
            writer.length("SemiAxis2", *semi_axis_2);
            "ellipse"
        }
        ProfileParameters::IShape {
            overall_width,
            overall_depth,
            web_thickness,
            flange_thickness,
            fillet_radius,
            flange_edge_radius,
            flange_slope,
            ..
        } => {
            writer.length("OverallWidth", *overall_width);
            writer.length("OverallDepth", *overall_depth);
            writer.length("WebThickness", *web_thickness);
            writer.length("FlangeThickness", *flange_thickness);
            writer.optional_length("FilletRadius", *fillet_radius);
            writer.optional_length("FlangeEdgeRadius", *flange_edge_radius);
            writer.optional_angle("FlangeSlope", *flange_slope);
            "i-shape"
        }
        ProfileParameters::AsymmetricIShape {
            bottom_flange_width,
            overall_depth,
            web_thickness,
            bottom_flange_thickness,
            bottom_flange_fillet_radius,
            top_flange_width,
            top_flange_thickness,
            top_flange_fillet_radius,
            bottom_flange_edge_radius,
            bottom_flange_slope,
            top_flange_edge_radius,
            top_flange_slope,
            ..
        } => {
            writer.length("BottomFlangeWidth", *bottom_flange_width);
            writer.length("OverallDepth", *overall_depth);
            writer.length("WebThickness", *web_thickness);
            writer.length("BottomFlangeThickness", *bottom_flange_thickness);
            writer.optional_length("BottomFlangeFilletRadius", *bottom_flange_fillet_radius);
            writer.length("TopFlangeWidth", *top_flange_width);
            writer.optional_length("TopFlangeThickness", *top_flange_thickness);
            writer.optional_length("TopFlangeFilletRadius", *top_flange_fillet_radius);
            writer.optional_length("BottomFlangeEdgeRadius", *bottom_flange_edge_radius);
            writer.optional_angle("BottomFlangeSlope", *bottom_flange_slope);
            writer.optional_length("TopFlangeEdgeRadius", *top_flange_edge_radius);
            writer.optional_angle("TopFlangeSlope", *top_flange_slope);
            "asymmetric-i-shape"
        }
        ProfileParameters::LShape {
            depth,
            width,
            thickness,
            fillet_radius,
            edge_radius,
            leg_slope,
            ..
        } => {
            writer.length("Depth", *depth);
            writer.optional_length("Width", *width);
            writer.length("Thickness", *thickness);
            writer.optional_length("FilletRadius", *fillet_radius);
            writer.optional_length("EdgeRadius", *edge_radius);
            writer.optional_angle("LegSlope", *leg_slope);
            "l-shape"
        }
        ProfileParameters::TShape {
            depth,
            flange_width,
            web_thickness,
            flange_thickness,
            fillet_radius,
            flange_edge_radius,
            web_edge_radius,
            web_slope,
            flange_slope,
            ..
        } => {
            writer.length("Depth", *depth);
            writer.length("FlangeWidth", *flange_width);
            writer.length("WebThickness", *web_thickness);
            writer.length("FlangeThickness", *flange_thickness);
            writer.optional_length("FilletRadius", *fillet_radius);
            writer.optional_length("FlangeEdgeRadius", *flange_edge_radius);
            writer.optional_length("WebEdgeRadius", *web_edge_radius);
            writer.optional_angle("WebSlope", *web_slope);
            writer.optional_angle("FlangeSlope", *flange_slope);
            "t-shape"
        }
        ProfileParameters::UShape {
            depth,
            flange_width,
            web_thickness,
            flange_thickness,
            fillet_radius,
            edge_radius,
            flange_slope,
            ..
        } => {
            writer.length("Depth", *depth);
            writer.length("FlangeWidth", *flange_width);
            writer.length("WebThickness", *web_thickness);
            writer.length("FlangeThickness", *flange_thickness);
            writer.optional_length("FilletRadius", *fillet_radius);
            writer.optional_length("EdgeRadius", *edge_radius);
            writer.optional_angle("FlangeSlope", *flange_slope);
            "u-shape"
        }
        ProfileParameters::CShape {
            depth,
            width,
            wall_thickness,
            girth,
            internal_fillet_radius,
            ..
        } => {
            writer.length("Depth", *depth);
            writer.length("Width", *width);
            writer.length("WallThickness", *wall_thickness);
            writer.length("Girth", *girth);
            writer.optional_length("InternalFilletRadius", *internal_fillet_radius);
            "c-shape"
        }
        ProfileParameters::ZShape {
            depth,
            flange_width,
            web_thickness,
            flange_thickness,
            fillet_radius,
            edge_radius,
            ..
        } => {
            writer.length("Depth", *depth);
            writer.length("FlangeWidth", *flange_width);
            writer.length("WebThickness", *web_thickness);
            writer.length("FlangeThickness", *flange_thickness);
            writer.optional_length("FilletRadius", *fillet_radius);
            writer.optional_length("EdgeRadius", *edge_radius);
            "z-shape"
        }
        ProfileParameters::Trapezium {
            bottom_x_dim,
            top_x_dim,
            y_dim,
            top_x_offset,
            ..
        } => {
            writer.length("BottomXDim", *bottom_x_dim);
            writer.length("TopXDim", *top_x_dim);
            writer.length("YDim", *y_dim);
            writer.length("TopXOffset", *top_x_offset);
            "trapezium"
        }
        ProfileParameters::ArbitraryClosed { .. } => {
            write_outline(writer, profile.entity, 0);
            "arbitrary-closed"
        }
        ProfileParameters::ArbitraryWithVoids { inner_curves, .. } => {
            writer.integer("VoidCount", inner_curves.len());
            write_outline(writer, profile.entity, inner_curves.len());
            "arbitrary-with-voids"
        }
        // A swept area never has an open profile: the description refuses it.
        ProfileParameters::ArbitraryOpen { .. } => "arbitrary-open",
        ProfileParameters::CenterLine { thickness, .. } => {
            writer.length("Thickness", *thickness);
            "center-line"
        }
        ProfileParameters::Composite {
            profiles, label, ..
        } => {
            writer.integer("Count", profiles.len());
            if let Some(label) = label {
                writer.text("Label", label);
            }
            for (index, member) in profiles.iter().enumerate() {
                let detail = format!("{}>{}", writer.detail, member.entity);
                write_profile(
                    &mut writer.nested(&format!("Member{}.", index + 1), detail),
                    member,
                );
            }
            "composite"
        }
        ProfileParameters::Derived { parent, label, .. } => {
            if let Some(label) = label {
                writer.text("Label", label);
            }
            let detail = format!("{}>{}", writer.detail, parent.entity);
            write_profile(&mut writer.nested("Parent.", detail), parent);
            "derived"
        }
        ProfileParameters::Mirrored { parent, label, .. } => {
            if let Some(label) = label {
                writer.text("Label", label);
            }
            let detail = format!("{}>{}", writer.detail, parent.entity);
            write_profile(&mut writer.nested("Parent.", detail), parent);
            "mirrored"
        }
        _ => {
            writer.put(
                "Type",
                Err(PropertyResolutionError::Unavailable(format!(
                    "{} is a profile family this adapter does not name yet",
                    profile.type_name
                ))),
            );
            return;
        }
    };
    writer.text("Type", family);
}

/// An arbitrary profile's outline as `OutlineX` and `OutlineY`, and each of
/// its `voids` as `Void<n>.OutlineX` and `Void<n>.OutlineY`: the vertices
/// of each ring in the order the file states them, the closing vertex not
/// repeated, in the profile's coordinates.
///
/// `ifc-geometry`'s `profile_outline` (openbimrs/ifc#166) reads them from
/// polylines and line-only indexed poly curves. A curved segment, or any
/// other curve family, cannot be stated as vertices: it refuses every
/// outline fact of the profile, never a chorded polygon, and leaves the
/// profile's other facts standing.
fn write_outline(writer: &mut Writer<'_>, profile: EntityId, voids: usize) {
    let mut names = vec![("OutlineX".to_owned(), "OutlineY".to_owned())];
    names.extend((1..=voids).map(|n| (format!("Void{n}.OutlineX"), format!("Void{n}.OutlineY"))));
    let outline = match profile_outline(writer.model, &writer.units.scale, profile) {
        Ok(outline) if outline.inner.len() == voids => outline,
        Ok(outline) => {
            let error = PropertyResolutionError::Conflicting(format!(
                "the outline of {profile} has {} voids, its description {voids}",
                outline.inner.len()
            ));
            for (x, y) in names {
                writer.put(&x, Err(error.clone()));
                writer.put(&y, Err(error.clone()));
            }
            return;
        }
        Err(error) => {
            let error = match error {
                GeometryError::Unsupported { .. } => PropertyResolutionError::Unavailable(format!(
                    "the outline cannot be stated as vertices: {error}"
                )),
                other => PropertyResolutionError::Incomplete(format!(
                    "the outline cannot be read: {other}"
                )),
            };
            for (x, y) in names {
                writer.put(&x, Err(error.clone()));
                writer.put(&y, Err(error.clone()));
            }
            return;
        }
    };
    let rings = std::iter::once(&outline.outer).chain(&outline.inner);
    for ((x, y), ring) in names.into_iter().zip(rings) {
        let coordinates = |axis: usize| {
            ring.iter()
                .map(|vertex| quantity(vertex[axis], QuantityDimension::Length))
                .collect::<Result<Vec<_>, _>>()
                .map(PropertyValue::List)
        };
        writer.put(&x, coordinates(0));
        writer.put(&y, coordinates(1));
    }
}
