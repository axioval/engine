//! Terrain surfaces as context: the ground a model stands on, supplied
//! beside the model rather than read from it.
//!
//! A terrain is one surface of each [`TerrainKind`]: the original ground
//! and the finished ground, each served when supplied and never one in
//! place of the other, so a rule names the one it measures against. A
//! surface states the coordinates it is given in ([`TerrainFrame`]): the
//! model's own, or the map the model's coordinate system maps onto. Placing
//! a model's plan point on a map surface is the caller's (the measured
//! values do it through the source's stated map conversion and refuse where
//! none is stated); a service answers only in its own coordinates.
//!
//! Every elevation is an interval sure to hold the terrain's true
//! elevation as far as its stated vertical accuracy holds: the surface's
//! own elevation widened by that accuracy on both sides, and by the
//! rounding of the interpolation. The accuracy is part of the evidence:
//! a surface states it ([`TerrainSurfaceStatement::accuracy_metres`]),
//! every answer carries it and the handle refuses an answer narrower than
//! twice it. An accuracy is never assumed: a source stating none is
//! refused unless its host states one, and only a stated zero makes an
//! answer exact.
//!
//! A plan point the surface does not cover is refused by name
//! ([`TerrainError::Outside`]), never answered with the nearest edge's
//! elevation, and so is a point the surface folds over with two different
//! elevations ([`TerrainError::Folded`]).

use std::collections::BTreeSet;
use std::sync::Arc;

use thiserror::Error;

mod landxml;
mod tin;

pub use tin::{TERRAIN_FILE_LIMIT_BYTES, TerrainSurface, TinTerrain};

/// The prefix of every terrain locator:
/// `axioval:terrain;kind=<kind>;sha256=<digest>#surface=<name>`.
pub const TERRAIN_LOCATOR_PREFIX: &str = "axioval:terrain";

/// Which ground a surface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TerrainKind {
    /// The ground before construction, as surveyed.
    Original,
    /// The ground after construction, as designed or surveyed.
    Finished,
}

impl TerrainKind {
    /// Every kind, in order.
    pub const ALL: [Self; 2] = [Self::Original, Self::Finished];

    /// `original` or `finished`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Finished => "finished",
        }
    }

    /// The kind named `name`, ignoring ASCII case.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(name))
    }
}

impl std::fmt::Display for TerrainKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The coordinates a surface is given in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TerrainFrame {
    /// The models' own coordinates, in metres: the coordinates their
    /// geometry is stated in.
    Model,
    /// Map coordinates (easting, northing, height), in metres: the map
    /// each model's stated map conversion maps onto.
    Map,
}

impl TerrainFrame {
    /// `model` or `map`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Map => "map",
        }
    }
}

/// Failure to read or serve a terrain.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum TerrainError {
    /// No surface of the kind is supplied.
    #[error("no {0} terrain is supplied")]
    NotSupplied(TerrainKind),
    /// The plan point lies outside the surface.
    #[error(
        "the plan point ({}, {}) lies outside the {kind} terrain surface `{surface}`",
        .point[0], .point[1]
    )]
    Outside {
        kind: TerrainKind,
        surface: String,
        /// The point, in the surface's coordinates.
        point: [f64; 2],
    },
    /// The surface holds the plan point at two different elevations.
    #[error(
        "the {kind} terrain surface `{surface}` folds over the plan point ({}, {}): it lies \
         there at {} and at {}",
        .point[0], .point[1], .elevations[0], .elevations[1]
    )]
    Folded {
        kind: TerrainKind,
        surface: String,
        point: [f64; 2],
        elevations: [f64; 2],
    },
    /// A surface's statements are malformed: no triangle, a point that is
    /// not finite, a face naming no point, an accuracy that is not a
    /// finite non-negative length.
    #[error("terrain surface is invalid: {0}")]
    Invalid(String),
    /// A terrain file cannot be read.
    #[error("terrain cannot be read: {0}")]
    Unreadable(String),
    /// The service cannot answer the request.
    #[error("terrain cannot be measured: {0}")]
    Unavailable(String),
    /// A request or an answer is malformed: a point that is not finite, an
    /// interval that is empty or narrower than the accuracy allows.
    #[error("terrain measurement is invalid")]
    InvalidMeasurement,
    /// The service answered another request, or with another accuracy than
    /// its surface states.
    #[error("terrain service answered another request")]
    ResponseRequestMismatch,
}

/// What a service states about one of its surfaces.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainSurfaceStatement {
    kind: TerrainKind,
    name: String,
    frame: TerrainFrame,
    crs: Option<String>,
    accuracy_metres: f64,
    locator: String,
}

impl TerrainSurfaceStatement {
    /// A statement. The accuracy must be a finite non-negative length and
    /// the locator non-blank; a named coordinate reference system belongs
    /// to a map surface only.
    pub fn try_new(
        kind: TerrainKind,
        name: impl Into<String>,
        frame: TerrainFrame,
        crs: Option<String>,
        accuracy_metres: f64,
        locator: impl Into<String>,
    ) -> Result<Self, TerrainError> {
        let locator = locator.into();
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(TerrainError::Invalid(format!(
                "a vertical accuracy of {accuracy_metres} m is not a non-negative length"
            )));
        }
        if locator.trim().is_empty() || (crs.is_some() && frame != TerrainFrame::Map) {
            return Err(TerrainError::InvalidMeasurement);
        }
        Ok(Self {
            kind,
            name: name.into(),
            frame,
            crs: crs.filter(|crs| !crs.trim().is_empty()),
            accuracy_metres,
            locator,
        })
    }

    /// Which ground the surface is.
    #[must_use]
    pub fn kind(&self) -> TerrainKind {
        self.kind
    }

    /// The surface's name in its source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The coordinates the surface is given in.
    #[must_use]
    pub fn frame(&self) -> TerrainFrame {
        self.frame
    }

    /// The map coordinate reference system the source names, if any.
    #[must_use]
    pub fn crs(&self) -> Option<&str> {
        self.crs.as_deref()
    }

    /// The stated vertical accuracy: the most any elevation of the
    /// surface may differ from the true ground, in metres.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }

    /// Where the surface comes from
    /// ([`TERRAIN_LOCATOR_PREFIX`]`;kind=…;sha256=…#surface=…`).
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
}

/// The elevation of one surface at a plan point, in its own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainPointRequest {
    kind: TerrainKind,
    point: [f64; 2],
}

impl TerrainPointRequest {
    /// A request; the point must be finite.
    pub fn try_new(kind: TerrainKind, point: [f64; 2]) -> Result<Self, TerrainError> {
        if !point.iter().all(|value| value.is_finite()) {
            return Err(TerrainError::InvalidMeasurement);
        }
        Ok(Self { kind, point })
    }

    /// The surface asked.
    #[must_use]
    pub fn kind(&self) -> TerrainKind {
        self.kind
    }

    /// The plan point, in the surface's coordinates.
    #[must_use]
    pub fn point(&self) -> [f64; 2] {
        self.point
    }
}

/// The mean elevation of one surface along closed plan rings, such as a
/// building's outline, in its own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainOutlineRequest {
    kind: TerrainKind,
    rings: Vec<Vec<[f64; 2]>>,
    deviation_metres: f64,
}

impl TerrainOutlineRequest {
    /// A request. Each ring is closed implicitly (its last point joins its
    /// first), has at least three finite points and a positive length;
    /// there is at least one ring. `deviation_metres` bounds how far the
    /// true outline may lie from the stated one in plan (a tessellated
    /// body's chord deviation), zero when the rings are the outline.
    pub fn try_new(
        kind: TerrainKind,
        rings: Vec<Vec<[f64; 2]>>,
        deviation_metres: f64,
    ) -> Result<Self, TerrainError> {
        let ring_valid = |ring: &Vec<[f64; 2]>| {
            ring.len() >= 3
                && ring.iter().flatten().all(|value| value.is_finite())
                && ring_length(ring) > 0.0
        };
        if rings.is_empty()
            || !rings.iter().all(ring_valid)
            || !deviation_metres.is_finite()
            || deviation_metres < 0.0
        {
            return Err(TerrainError::InvalidMeasurement);
        }
        Ok(Self {
            kind,
            rings,
            deviation_metres,
        })
    }

    /// The surface asked.
    #[must_use]
    pub fn kind(&self) -> TerrainKind {
        self.kind
    }

    /// The rings, in the surface's coordinates.
    #[must_use]
    pub fn rings(&self) -> &[Vec<[f64; 2]>] {
        &self.rings
    }

    /// How far the true outline may lie from the rings in plan.
    #[must_use]
    pub fn deviation_metres(&self) -> f64 {
        self.deviation_metres
    }
}

/// The plan length of a closed ring.
pub(crate) fn ring_length(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .sum()
}

/// A terrain elevation: an interval sure to hold the true ground's
/// elevation as far as the surface's stated accuracy holds.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainElevation {
    kind: TerrainKind,
    lower_metres: f64,
    upper_metres: f64,
    accuracy_metres: f64,
    exact: bool,
    locator: String,
}

impl TerrainElevation {
    /// An elevation. The bounds must be finite and ordered, and at least
    /// twice the accuracy apart, since the accuracy widens both sides; it
    /// may be exact only with an accuracy of zero.
    pub fn try_new(
        kind: TerrainKind,
        (lower_metres, upper_metres): (f64, f64),
        accuracy_metres: f64,
        exact: bool,
        locator: impl Into<String>,
    ) -> Result<Self, TerrainError> {
        let locator = locator.into();
        let valid = lower_metres.is_finite()
            && upper_metres.is_finite()
            && lower_metres <= upper_metres
            && accuracy_metres.is_finite()
            && accuracy_metres >= 0.0
            && covers_accuracy(lower_metres, upper_metres, accuracy_metres)
            && (!exact || accuracy_metres == 0.0)
            && !locator.trim().is_empty();
        if !valid {
            return Err(TerrainError::InvalidMeasurement);
        }
        Ok(Self {
            kind,
            lower_metres,
            upper_metres,
            accuracy_metres,
            exact,
            locator,
        })
    }

    /// The surface measured.
    #[must_use]
    pub fn kind(&self) -> TerrainKind {
        self.kind
    }

    /// The least elevation the ground may have.
    #[must_use]
    pub fn lower_metres(&self) -> f64 {
        self.lower_metres
    }

    /// The greatest elevation the ground may have.
    #[must_use]
    pub fn upper_metres(&self) -> f64 {
        self.upper_metres
    }

    /// The surface's stated vertical accuracy the interval includes.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }

    /// Whether the interval holds only the rounding of exact arithmetic
    /// over a surface stated exact.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Where the elevation comes from.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
}

/// Whether `[lower, upper]` is at least twice `accuracy` wide, but for the
/// rounding of the widening itself.
fn covers_accuracy(lower: f64, upper: f64, accuracy: f64) -> bool {
    let scale = lower.abs().max(upper.abs()).max(1.0);
    upper - lower >= 2.0 * accuracy - 8.0 * f64::EPSILON * scale
}

/// Trusted seam serving terrain surfaces.
pub trait TerrainService: Send + Sync + 'static {
    /// The surfaces served, at most one of each kind.
    // gate: reads
    fn surfaces(&self) -> Vec<TerrainSurfaceStatement>;

    /// The elevation of the request's surface at its plan point.
    // gate: measures length
    fn measure_elevation(
        &self,
        request: &TerrainPointRequest,
    ) -> Result<TerrainElevation, TerrainError>;

    /// The mean elevation of the request's surface along its rings,
    /// weighted by plan length.
    ///
    /// A service that does not measure along outlines refuses; it never
    /// answers with the elevation at one point.
    // gate: measures length
    fn measure_mean_elevation(
        &self,
        request: &TerrainOutlineRequest,
    ) -> Result<TerrainElevation, TerrainError> {
        let _ = request;
        Err(TerrainError::Unavailable(
            "this terrain service does not measure along outlines".into(),
        ))
    }
}

/// The grounds `plan` may measure against: the `terrain` of every
/// `above_terrain` and `mean_terrain_height` its rules and values read, and
/// both grounds where a rule binds it from a parameter. A host warns of a
/// ground read but not supplied, whose values are not evaluated.
#[must_use]
pub fn terrain_kinds_read(plan: &crate::ExecutionPlan) -> BTreeSet<TerrainKind> {
    fn walk(value: &serde_json::Value, found: &mut BTreeSet<TerrainKind>) {
        match value {
            serde_json::Value::String(text) => {
                let lowered = text.trim_start().to_ascii_lowercase();
                if !(lowered.starts_with("above_terrain")
                    || lowered.starts_with("mean_terrain_height"))
                {
                    return;
                }
                let Ok(call) = axioval_ir::measured::parse(text) else {
                    return;
                };
                match call.choice("terrain").and_then(TerrainKind::parse) {
                    Some(kind) => {
                        found.insert(kind);
                    }
                    None => found.extend(TerrainKind::ALL),
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            serde_json::Value::Object(fields) => {
                for item in fields.values() {
                    walk(item, found);
                }
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    for rule in &plan.rules {
        for value in [
            serde_json::to_value(&rule.parameters),
            serde_json::to_value(&rule.selector),
        ]
        .into_iter()
        .flatten()
        {
            walk(&value, &mut found);
        }
    }
    if let Ok(values) = serde_json::to_value(&*plan.values) {
        walk(&values, &mut found);
    }
    found
}

/// Registry handle for a [`TerrainService`].
#[derive(Clone)]
pub struct TerrainServiceHandle(Arc<dyn TerrainService>);

impl TerrainServiceHandle {
    /// Wraps a trusted terrain service.
    #[must_use]
    pub fn new(service: Arc<dyn TerrainService>) -> Self {
        Self(service)
    }

    /// The surfaces served, by kind.
    #[must_use]
    pub fn surfaces(&self) -> Vec<TerrainSurfaceStatement> {
        let mut surfaces = self.0.surfaces();
        surfaces.sort_by_key(TerrainSurfaceStatement::kind);
        surfaces
    }

    /// What the service states about its surface of `kind`; a kind it
    /// does not serve is [`TerrainError::NotSupplied`].
    pub fn surface(&self, kind: TerrainKind) -> Result<TerrainSurfaceStatement, TerrainError> {
        let mut found = self.0.surfaces().into_iter().filter(|s| s.kind() == kind);
        match (found.next(), found.next()) {
            (Some(surface), None) => Ok(surface),
            (None, _) => Err(TerrainError::NotSupplied(kind)),
            (Some(_), Some(_)) => Err(TerrainError::Unavailable(format!(
                "two {kind} terrain surfaces are served"
            ))),
        }
    }

    /// The elevation at the request's point. A surface not served is
    /// refused before the service is asked, and so is an answer about
    /// another surface or with another accuracy than the surface states.
    pub fn measure_elevation(
        &self,
        request: &TerrainPointRequest,
    ) -> Result<TerrainElevation, TerrainError> {
        let surface = self.surface(request.kind())?;
        let answer = self.0.measure_elevation(request)?;
        Self::check(&surface, &answer)?;
        Ok(answer)
    }

    /// The mean elevation along the request's rings, checked as
    /// [`Self::measure_elevation`] checks an elevation; an exact answer
    /// needs an outline stated exactly too.
    pub fn measure_mean_elevation(
        &self,
        request: &TerrainOutlineRequest,
    ) -> Result<TerrainElevation, TerrainError> {
        let surface = self.surface(request.kind())?;
        let answer = self.0.measure_mean_elevation(request)?;
        Self::check(&surface, &answer)?;
        if answer.is_exact() && request.deviation_metres() > 0.0 {
            return Err(TerrainError::InvalidMeasurement);
        }
        Ok(answer)
    }

    fn check(
        surface: &TerrainSurfaceStatement,
        answer: &TerrainElevation,
    ) -> Result<(), TerrainError> {
        #[allow(clippy::float_cmp)] // The accuracy is stated, never computed.
        if answer.kind() != surface.kind() || answer.accuracy_metres() != surface.accuracy_metres()
        {
            return Err(TerrainError::ResponseRequestMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(TerrainElevation, f64);

    impl TerrainService for Fixed {
        fn surfaces(&self) -> Vec<TerrainSurfaceStatement> {
            vec![
                TerrainSurfaceStatement::try_new(
                    TerrainKind::Original,
                    "g",
                    TerrainFrame::Model,
                    None,
                    self.1,
                    "axioval:terrain;kind=original",
                )
                .unwrap(),
            ]
        }

        fn measure_elevation(
            &self,
            _: &TerrainPointRequest,
        ) -> Result<TerrainElevation, TerrainError> {
            Ok(self.0.clone())
        }
    }

    fn elevation(kind: TerrainKind, accuracy: f64) -> TerrainElevation {
        TerrainElevation::try_new(
            kind,
            (10.0 - accuracy, 10.0 + accuracy),
            accuracy,
            accuracy == 0.0,
            "l",
        )
        .unwrap()
    }

    #[test]
    fn an_elevation_is_at_least_as_wide_as_its_accuracy_and_exact_only_without_one() {
        assert!(
            TerrainElevation::try_new(TerrainKind::Original, (9.9, 10.1), 0.1, false, "l").is_ok()
        );
        for (bounds, accuracy, exact) in [
            ((9.95, 10.05), 0.1, false),
            ((9.9, 10.1), 0.1, true),
            ((10.1, 9.9), 0.0, false),
            ((9.9, 10.1), -0.1, false),
            ((f64::NAN, 10.1), 0.0, false),
        ] {
            assert_eq!(
                TerrainElevation::try_new(TerrainKind::Original, bounds, accuracy, exact, "l"),
                Err(TerrainError::InvalidMeasurement),
                "{bounds:?} {accuracy} {exact}"
            );
        }
    }

    #[test]
    fn the_handle_refuses_a_surface_not_served_and_an_answer_of_another_accuracy() {
        let handle =
            TerrainServiceHandle::new(Arc::new(Fixed(elevation(TerrainKind::Original, 0.1), 0.1)));
        let at = |kind| TerrainPointRequest::try_new(kind, [0.0, 0.0]).unwrap();
        assert!(handle.measure_elevation(&at(TerrainKind::Original)).is_ok());
        assert_eq!(
            handle.measure_elevation(&at(TerrainKind::Finished)),
            Err(TerrainError::NotSupplied(TerrainKind::Finished))
        );
        let narrower =
            TerrainServiceHandle::new(Arc::new(Fixed(elevation(TerrainKind::Original, 0.05), 0.1)));
        assert_eq!(
            narrower.measure_elevation(&at(TerrainKind::Original)),
            Err(TerrainError::ResponseRequestMismatch)
        );
        let outline = TerrainOutlineRequest::try_new(
            TerrainKind::Original,
            vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            0.0,
        )
        .unwrap();
        assert!(matches!(
            handle.measure_mean_elevation(&outline),
            Err(TerrainError::Unavailable(_))
        ));
    }

    #[test]
    fn a_request_needs_finite_points_and_closed_rings() {
        assert!(TerrainPointRequest::try_new(TerrainKind::Original, [f64::INFINITY, 0.0]).is_err());
        for rings in [
            vec![],
            vec![vec![[0.0, 0.0], [1.0, 0.0]]],
            vec![vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]],
        ] {
            assert!(TerrainOutlineRequest::try_new(TerrainKind::Original, rings, 0.0).is_err());
        }
        assert!(
            TerrainOutlineRequest::try_new(
                TerrainKind::Original,
                vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
                -1.0
            )
            .is_err()
        );
    }

    #[test]
    fn kinds_parse_ignoring_case() {
        assert_eq!(TerrainKind::parse("Finished"), Some(TerrainKind::Finished));
        assert_eq!(TerrainKind::parse("existing"), None);
    }
}
