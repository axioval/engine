//! Overlays held to the overlay budgets (#432, #433).
//!
//! [`super::plan_overlay`] bounds the overlays of polygon sets and
//! [`soup_union`] the unions of ring soups, both through [`within`]: the
//! pairs of meeting edges are counted against `OVERLAY_PAIRS` before the
//! kernel is called, and the kernel builds the subdivision under a memory
//! budget of its own (`OVERLAY_MEMORY`, or the check's memory budget where
//! that is smaller), refusing it by name before allocating past it.
//! The kernel has no budgeted call for a boolean of two `Region`s or a
//! one-sided disc morphology: those count what they hand over against
//! `REGION_PAIRS` and `DISC_PAIRS` and refuse by name past them or where
//! their memory estimate alone is more than the check's memory budget.
//! Each is built while admitted against the check's memory budget. A
//! refusal is a function of the operands alone (and the check's memory
//! budget), so it is the same whether a result built from them was kept
//! or built again (ADR 0006).

use axiolid_contracts::ExecutionOptions;
use axiolid_core::Tolerance;
use axiolid_overlay::{OverlayOptions, OverlayRefusal, Polygon, Region, Ring, union_soup_within};
use axioval_engine::memory::{self, Admission};

use super::PlanarRefusal;
use super::budget::{self, Counted, OVERLAY_MEMORY, Step};

/// Builds an overlay the kernel holds to a memory budget (#433): `build`
/// is handed the execution options to call it with, at `tolerance`.
///
/// `counted` is what the overlay hands over, within `OVERLAY_PAIRS`
/// ([`budget::count`]). The cap is the smaller of [`OVERLAY_MEMORY`] and
/// the check's memory budget. The overlay is first built under
/// [`Counted::attempt`]; refused there, it is built again under four
/// times what the kernel had charged (at least twice the budget before),
/// up to the cap, and refused past the cap by name
/// ([`PlanarRefusal::Subdivision`]). Each attempt is admitted against the
/// check's memory budget with what the kernel does not charge beside it.
///
/// The kernel's charges do not depend on its budget, so an admitted
/// overlay is the same under any budget, and a refusal comes at the first
/// charge past it. One past the cap is therefore refused at once, as the
/// cap would refuse it, with the same charge: whether an overlay is built,
/// and the refusal's numbers, depend on its operands and the cap alone,
/// never on the attempts, on what else runs or on what was kept.
pub(crate) fn within<T>(
    counted: Counted,
    tolerance: Tolerance,
    build: impl Fn(&ExecutionOptions) -> Result<T, OverlayRefusal>,
) -> Result<T, PlanarRefusal> {
    let limit = memory::current().map(|budget| budget.limit());
    let cap = limit.map_or(OVERLAY_MEMORY, |limit| limit.min(OVERLAY_MEMORY));
    let mut attempt = counted.attempt(cap);
    loop {
        // What the kernel may hold, within the check's budget: the
        // admission never refuses, it only waits for room.
        let held = attempt.saturating_add(counted.uncharged());
        let _admission = memory::admit(limit.map_or(held, |limit| held.min(limit)))
            .map_err(PlanarRefusal::Memory)?;
        let execution = ExecutionOptions::new(tolerance)
            .with_memory_budget(usize::try_from(attempt).unwrap_or(usize::MAX));
        match build(&execution) {
            Ok(overlay) => return Ok(overlay),
            Err(OverlayRefusal::Invalid(error)) => return Err(PlanarRefusal::Overlay(error)),
            Err(OverlayRefusal::BudgetExceeded { needed, .. }) => {
                let charged = u64::try_from(needed).unwrap_or(u64::MAX);
                if attempt >= cap || charged > cap {
                    return Err(PlanarRefusal::Subdivision(OverlayRefusal::BudgetExceeded {
                        needed,
                        budget: usize::try_from(cap).unwrap_or(usize::MAX),
                    }));
                }
                attempt = charged
                    .saturating_mul(4)
                    .max(attempt.saturating_mul(2))
                    .min(cap);
            }
            Err(refusal) => return Err(PlanarRefusal::Subdivision(refusal)),
        }
    }
}

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

/// The union of a soup of rings (`axiolid_overlay::union_soup_within`),
/// refused before it is built where their edges meet in more pairs than
/// the overlay budget allows, and by the kernel where its subdivision
/// needs more than the overlay memory budget ([`within`]). It is
/// `union_soup` exactly where admitted.
pub(crate) fn soup_union(
    rings: &[Ring],
    tolerance: Tolerance,
) -> Result<Vec<Polygon>, PlanarRefusal> {
    if rings.is_empty() {
        return Ok(Vec::new());
    }
    let counted = budget::count_rings(rings).map_err(PlanarRefusal::OverBudget)?;
    within(counted, tolerance, |execution| {
        union_soup_within(rings, OverlayOptions::new(), execution).map(|union| union.polygons)
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
    use std::sync::Arc;

    use axiolid_core::{Point2, Tolerance};
    use axiolid_overlay::{
        FillRule, OverlayInput, OverlayOperation, OverlayRefusal, Polygon, Region, Ring,
    };
    use axioval_engine::MemoryBudget;

    use super::{Boolean, Morphology, combine, morphology, soup_union};
    use crate::planar::budget::{DISC_PAIRS, OVERLAY_PAIRS, Refusal};
    use crate::planar::{PlanarRefusal, plan_overlay};

    /// 4,000 long, thin triangles along one line, each meeting every
    /// other: 12,000 edges in about 64 million pairs, past the overlay
    /// budget on their own (and so past the region boolean's).
    pub(crate) fn slivers() -> Vec<Ring> {
        (0..4000)
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

    /// `teeth` strips 4 m long and 1 cm wide at a 5 cm pitch, along x or
    /// along y, as two triangles each: two such combs cross in a grid.
    fn comb(teeth: u32, along_x: bool) -> Vec<Ring> {
        let mut rings = Vec::new();
        for tooth in 0..teeth {
            let offset = 0.05 * f64::from(tooth);
            let (x0, y0, x1, y1) = if along_x {
                (0.0, offset, 4.0, offset + 0.01)
            } else {
                (offset, 0.0, offset + 0.01, 4.0)
            };
            let p = Point2::new;
            rings.push(Ring {
                points: vec![p(x0, y0), p(x1, y0), p(x1, y1)],
            });
            rings.push(Ring {
                points: vec![p(x0, y0), p(x1, y1), p(x0, y1)],
            });
        }
        rings
    }

    fn input(rings: Vec<Ring>) -> OverlayInput {
        OverlayInput {
            frame: crate::planar::plan_frame(),
            polygons: polygons(rings),
        }
    }

    /// Within the overlay budget, a plan overlay and a ring-soup union
    /// are built under the kernel's memory budget exactly as without one;
    /// where the subdivision needs more than the check's memory budget,
    /// the kernel refuses it by name before allocating past it, alike
    /// however often it is asked (#433).
    #[test]
    fn the_kernel_refuses_a_subdivision_past_the_memory_budget_by_name() {
        let t = tolerance();
        let (rows, columns) = (input(comb(40, true)), input(comb(40, false)));
        let mut soup = comb(40, true);
        soup.extend(comb(40, false));
        let overlay = || {
            plan_overlay(
                &rows,
                &columns,
                OverlayOperation::Intersection,
                FillRule::NonZero,
                t,
            )
        };
        let built = overlay().unwrap();
        assert_eq!(
            built,
            axiolid_overlay::overlay(
                &rows,
                &columns,
                OverlayOperation::Intersection,
                FillRule::NonZero,
                t
            )
            .unwrap()
        );
        assert_eq!(built.polygons.len(), 1600);
        assert_eq!(
            soup_union(&soup, t).unwrap(),
            axiolid_overlay::union_soup(&soup, t).unwrap()
        );
        let budget = Arc::new(MemoryBudget::new(8 << 20));
        let _entered = budget.enter();
        let refused = overlay().unwrap_err();
        assert!(
            matches!(
                refused,
                PlanarRefusal::Subdivision(OverlayRefusal::BudgetExceeded { budget, .. })
                    if budget == 8 << 20
            ),
            "{refused:?}"
        );
        assert!(refused.is_budget());
        assert!(
            refused.to_string().starts_with(
                "the plan overlay was not built: its subdivision needs more than the overlay \
                 memory budget of 8.0 MiB ("
            ),
            "{refused}"
        );
        // Refused as the kernel refuses it under the cap at once, though
        // first tried under less, and alike when asked again.
        let direct = axiolid_overlay::overlay_within(
            &rows,
            &columns,
            OverlayOperation::Intersection,
            FillRule::NonZero,
            axiolid_overlay::OverlayOptions::new(),
            &axiolid_contracts::ExecutionOptions::new(t).with_memory_budget(8 << 20),
        )
        .unwrap_err();
        assert_eq!(refused, PlanarRefusal::Subdivision(direct));
        assert_eq!(overlay().unwrap_err(), refused);
        assert!(matches!(
            soup_union(&soup, t).unwrap_err(),
            PlanarRefusal::Subdivision(OverlayRefusal::BudgetExceeded { .. })
        ));
        assert_eq!(budget.in_use(), 0);
        // Small overlays are built under the same budget.
        let within = combine(&square(2.0), &square(1.0), Boolean::Difference, t).unwrap();
        assert!((within.area() - 3.0).abs() < 1e-6, "{}", within.area());
        assert_eq!(soup_union(&comb(2, true), t).unwrap().len(), 2);
    }
}
