//! Region operations held to the overlay budget (#432).
//!
//! [`super::plan_overlay`] bounds the overlays of polygon sets; these bound
//! the other ways the adapter hands the overlay an arrangement: a boolean
//! of two `Region`s, a union of a ring soup and a one-sided disc
//! morphology. Each counts what it hands over before anything is built
//! (`budget.rs`), refuses by name past its budget or where its memory
//! estimate alone is more than the check's memory budget, and is built
//! while admitted against that budget. A refusal is a function of the
//! operands alone, so it is the same whether a result built from them was
//! kept or built again (ADR 0006).

use axiolid_core::Tolerance;
use axiolid_overlay::{Polygon, Region, Ring, union_soup};
use axioval_engine::memory::{self, Admission};

use super::PlanarRefusal;
use super::budget::{self, Step};

/// An overlay admitted against the check's memory budget, built while
/// this is held; nothing to hold where nothing is built.
#[must_use = "an overlay is admitted only while this is held"]
#[derive(Debug)]
pub(crate) struct Admitted {
    _admission: Option<Admission>,
}

/// Admits a boolean of the regions `a` and `b`, or refuses it past the
/// overlay budget or the memory budget. An empty operand builds nothing
/// (the overlay answers the set identities itself), so it is admitted at
/// once.
pub(crate) fn admit_regions(a: &Region, b: &Region) -> Result<Admitted, PlanarRefusal> {
    if a.is_empty() || b.is_empty() {
        return Ok(Admitted { _admission: None });
    }
    let estimate =
        budget::admit(&[a.polygons(), b.polygons()]).map_err(PlanarRefusal::OverBudget)?;
    let admission = memory::admit(estimate).map_err(PlanarRefusal::Memory)?;
    Ok(Admitted {
        _admission: admission,
    })
}

/// A boolean of two regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boolean {
    /// [`Region::union`].
    Union,
    /// [`Region::intersection`].
    Intersection,
    /// [`Region::difference`].
    Difference,
}

/// `a` combined with `b` by `operation`, refused before it is built past
/// the overlay budget ([`admit_regions`]).
pub(crate) fn combine(
    a: &Region,
    b: &Region,
    operation: Boolean,
    tolerance: Tolerance,
) -> Result<Region, PlanarRefusal> {
    let _admitted = admit_regions(a, b)?;
    match operation {
        Boolean::Union => a.union(b, tolerance),
        Boolean::Intersection => a.intersection(b, tolerance),
        Boolean::Difference => a.difference(b, tolerance),
    }
    .map_err(PlanarRefusal::Overlay)
}

/// [`combine`] by [`Boolean::Union`].
pub(crate) fn union(a: &Region, b: &Region, tolerance: Tolerance) -> Result<Region, PlanarRefusal> {
    combine(a, b, Boolean::Union, tolerance)
}

/// [`combine`] by [`Boolean::Intersection`].
pub(crate) fn intersection(
    a: &Region,
    b: &Region,
    tolerance: Tolerance,
) -> Result<Region, PlanarRefusal> {
    combine(a, b, Boolean::Intersection, tolerance)
}

/// [`combine`] by [`Boolean::Difference`].
pub(crate) fn difference(
    a: &Region,
    b: &Region,
    tolerance: Tolerance,
) -> Result<Region, PlanarRefusal> {
    combine(a, b, Boolean::Difference, tolerance)
}

/// The union of a soup of rings (`axiolid_overlay::union_soup`), refused
/// before it is built where their edges meet in more pairs than the
/// overlay budget allows or its estimate is more than the memory budget.
pub(crate) fn soup_union(
    rings: &[Ring],
    tolerance: Tolerance,
) -> Result<Vec<Polygon>, PlanarRefusal> {
    let _admitted = admit_rings(rings)?;
    union_soup(rings, tolerance).map_err(PlanarRefusal::Overlay)
}

/// Admits a union of the ring soup `rings`, as [`soup_union`] builds it.
pub(crate) fn admit_rings(rings: &[Ring]) -> Result<Admitted, PlanarRefusal> {
    if rings.is_empty() {
        return Ok(Admitted { _admission: None });
    }
    let estimate = budget::admit_rings(rings).map_err(PlanarRefusal::OverBudget)?;
    let admission = memory::admit(estimate).map_err(PlanarRefusal::Memory)?;
    Ok(Admitted {
        _admission: admission,
    })
}

/// A one-sided disc morphology of a region in the overlay's exact mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Morphology {
    /// [`Region::dilate_inner`].
    DilateInner,
    /// [`Region::dilate_outer`].
    DilateOuter,
    /// [`Region::erode_inner`].
    ErodeInner,
    /// [`Region::erode_outer`].
    ErodeOuter,
}

impl Morphology {
    fn erodes(self) -> bool {
        matches!(self, Self::ErodeInner | Self::ErodeOuter)
    }

    fn what(self) -> &'static str {
        if self.erodes() {
            "erosion by a disc"
        } else {
            "dilation by a disc"
        }
    }
}

/// `region` grown or shrunk by a disc of `radius` (`kind`), refused before
/// it is built past the disc morphology budget (`budget::disc_cost`
/// against `DISC_PAIRS`) or where its estimate is more than the memory
/// budget. An empty region builds nothing.
pub(crate) fn morphology(
    region: &Region,
    kind: Morphology,
    radius: f64,
    tolerance: Tolerance,
) -> Result<Region, PlanarRefusal> {
    let _step = if region.is_empty() {
        Step::NONE
    } else {
        budget::admit_step(
            kind.what(),
            budget::disc_cost(region.polygons(), radius, kind.erodes()),
        )
        .map_err(PlanarRefusal::Morphology)?
    };
    match kind {
        Morphology::DilateInner => region.dilate_inner(radius, tolerance),
        Morphology::DilateOuter => region.dilate_outer(radius, tolerance),
        Morphology::ErodeInner => region.erode_inner(radius, tolerance),
        Morphology::ErodeOuter => region.erode_outer(radius, tolerance),
    }
    .map_err(PlanarRefusal::Overlay)
}

#[cfg(test)]
pub(crate) mod tests {
    use axiolid_core::{Point2, Tolerance};
    use axiolid_overlay::{Polygon, Region, Ring};

    use super::{Boolean, Morphology, combine, morphology, soup_union};
    use crate::planar::PlanarRefusal;
    use crate::planar::budget::{DISC_PAIRS, OVERLAY_PAIRS, Refusal};

    /// 3,000 long, thin triangles along one line, each meeting every
    /// other: 9,000 edges in about 4.5 million pairs, past the overlay
    /// budget on their own.
    pub(crate) fn slivers() -> Vec<Ring> {
        (0..3000)
            .map(|index| {
                let y = f64::from(index) * 1e-4;
                Ring {
                    points: vec![
                        Point2::new(0.0, y),
                        Point2::new(10.0, y + 0.5),
                        Point2::new(0.0, y + 1.0),
                    ],
                }
            })
            .collect()
    }

    /// [`slivers`] as flat triangles at the origin's level, as a mesh's
    /// shadows past the overlay budget.
    pub(crate) fn sliver_triangles() -> Vec<crate::geometry::Triangle> {
        slivers()
            .into_iter()
            .map(|ring| {
                let corner = |index: usize| {
                    let point = ring.points[index];
                    axiolid_core::Point3::new(point.x, point.y, 0.0)
                };
                [corner(0), corner(1), corner(2)]
            })
            .collect()
    }

    /// [`slivers`] as polygons without holes.
    pub(crate) fn sliver_polygons() -> Vec<Polygon> {
        polygons(slivers())
    }

    fn polygons(rings: Vec<Ring>) -> Vec<Polygon> {
        rings
            .into_iter()
            .map(|outer| Polygon {
                outer,
                holes: Vec::new(),
            })
            .collect()
    }

    fn tolerance() -> Tolerance {
        Tolerance::new(1e-9, 1e-9).unwrap()
    }

    fn square(side: f64) -> Region {
        let p = Point2::new;
        Region::new(
            polygons(vec![Ring {
                points: vec![p(0.0, 0.0), p(side, 0.0), p(side, side), p(0.0, side)],
            }]),
            tolerance(),
        )
        .unwrap()
    }

    /// Every region operation over operands past its budget is refused by
    /// name before it is built; one within it is built, and an empty
    /// operand builds nothing and is never refused.
    #[test]
    fn region_operations_past_the_budget_are_refused_by_name() {
        let t = tolerance();
        let crowded = Region::new(polygons(slivers()), t).unwrap();
        let refused = soup_union(&slivers(), t).unwrap_err();
        assert!(
            matches!(refused, PlanarRefusal::OverBudget(over) if over.pairs > OVERLAY_PAIRS),
            "{refused:?}"
        );
        assert!(refused.is_budget());
        assert!(
            refused
                .described()
                .ends_with(&format!("past the overlay budget of {OVERLAY_PAIRS} pairs")),
            "{}",
            refused.described()
        );
        for operation in [Boolean::Union, Boolean::Intersection, Boolean::Difference] {
            let refused = combine(&square(10.0), &crowded, operation, t).unwrap_err();
            assert!(
                matches!(refused, PlanarRefusal::OverBudget(_)),
                "{operation:?}: {refused:?}"
            );
            assert_eq!(
                combine(&Region::empty(), &crowded, operation, t)
                    .unwrap()
                    .is_empty(),
                operation != Boolean::Union
            );
        }
        let within = combine(&square(2.0), &square(1.0), Boolean::Difference, t).unwrap();
        assert!((within.area() - 3.0).abs() < 1e-6, "{}", within.area());
        for kind in [
            Morphology::DilateInner,
            Morphology::DilateOuter,
            Morphology::ErodeInner,
            Morphology::ErodeOuter,
        ] {
            let refused = morphology(&crowded, kind, 0.5, t).unwrap_err();
            assert!(
                matches!(
                    refused,
                    PlanarRefusal::Morphology(Refusal::Step { cost, .. }) if cost.pairs > DISC_PAIRS
                ),
                "{kind:?}: {refused:?}"
            );
            assert!(
                refused.to_string().ends_with(&format!(
                    "past the disc morphology budget of {DISC_PAIRS} pairs"
                )),
                "{refused}"
            );
            assert!(
                morphology(&Region::empty(), kind, 0.5, t)
                    .unwrap()
                    .is_empty()
            );
        }
        let grown = morphology(&square(1.0), Morphology::DilateOuter, 0.5, t).unwrap();
        assert!(grown.area() > 1.0 + 4.0 * 0.5, "{}", grown.area());
    }
}
