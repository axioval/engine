//! Broad-phase candidate search for pairwise spatial checks.
//!
//! Measuring every subject against every counterpart is quadratic in model
//! size and almost entirely wasted: most pairs are metres apart. This search
//! discards pairs by their enclosing boxes and hands only the rest to a
//! narrow-phase measurement.
//!
//! Discarding must never lose a real pair, so it works on
//! [`ObjectBounds::enclosing`] boxes, which contain the true body even for
//! tessellated geometry, and on the box gap, which never exceeds the gap
//! between the bodies inside. The result is therefore complete: a pair absent
//! from it is proven farther apart than the margin.

use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::ObjectId;

use crate::proximity::{Bounds3, ObjectBounds, ProximityProjection, VerticalDirection};

/// Why a candidate search could not run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CandidateSearchError {
    /// The search margin is negative or non-finite.
    #[error("candidate search margin must be finite and non-negative")]
    InvalidMargin,
    /// One object was supplied twice with different extents.
    #[error("object {0} was supplied with conflicting bounds")]
    ConflictingBounds(ObjectId),
}

/// A subject and a counterpart whose enclosing boxes lie within the margin.
///
/// When both objects belong to both groups the pair is reported once, with
/// the lesser identity as the subject.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidatePair {
    subject: ObjectId,
    counterpart: ObjectId,
}

impl CandidatePair {
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    pub fn counterpart(&self) -> &ObjectId {
        &self.counterpart
    }
}

struct Entry<'a> {
    id: &'a ObjectId,
    enclosing: Bounds3,
    subject: bool,
    counterpart: bool,
}

/// Every subject/counterpart pair whose enclosing boxes lie within
/// `margin_metres` of each other, in identity order.
///
/// An object may appear in both groups; it is never paired with itself.
/// Sweep-and-prune along x keeps the cost near-linear in the object count plus
/// the number of pairs reported.
pub fn candidate_pairs(
    subjects: &[ObjectBounds],
    counterparts: &[ObjectBounds],
    margin_metres: f64,
) -> Result<Vec<CandidatePair>, CandidateSearchError> {
    if !margin_metres.is_finite() || margin_metres < 0.0 {
        return Err(CandidateSearchError::InvalidMargin);
    }
    let mut entries: BTreeMap<&ObjectId, Entry<'_>> = BTreeMap::new();
    for (bounds, is_subject) in subjects
        .iter()
        .map(|b| (b, true))
        .chain(counterparts.iter().map(|b| (b, false)))
    {
        let entry = entries.entry(bounds.object()).or_insert_with(|| Entry {
            id: bounds.object(),
            enclosing: bounds.enclosing(),
            subject: false,
            counterpart: false,
        });
        if entry.enclosing != bounds.enclosing() {
            return Err(CandidateSearchError::ConflictingBounds(
                bounds.object().clone(),
            ));
        }
        if is_subject {
            entry.subject = true;
        } else {
            entry.counterpart = true;
        }
    }

    // Sort by the lower x of each box grown by the margin, identity breaking
    // ties so the sweep itself is deterministic.
    let mut sweep: Vec<Entry<'_>> = entries.into_values().collect();
    sweep.sort_by(|a, b| {
        a.enclosing.min()[0]
            .total_cmp(&b.enclosing.min()[0])
            .then_with(|| a.id.cmp(b.id))
    });

    let mut pairs = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    for (index, entry) in sweep.iter().enumerate() {
        // Anything whose x-extent ends more than the margin before this box
        // begins can meet nothing later in the sweep either.
        active.retain(|&open| {
            sweep[open].enclosing.max()[0] + margin_metres >= entry.enclosing.min()[0]
        });
        for &open in &active {
            let other = &sweep[open];
            let eligible =
                (entry.subject && other.counterpart) || (entry.counterpart && other.subject);
            if !eligible || entry.enclosing.gap(&other.enclosing) > margin_metres {
                continue;
            }
            let (first, second) = if entry.id < other.id {
                (entry, other)
            } else {
                (other, entry)
            };
            // Prefer the lesser identity as subject whenever that orientation
            // is allowed, so a symmetric search reports each pair once.
            let (subject, counterpart) = if first.subject && second.counterpart {
                (first.id, second.id)
            } else {
                (second.id, first.id)
            };
            pairs.push(CandidatePair {
                subject: subject.clone(),
                counterpart: counterpart.clone(),
            });
        }
        active.push(index);
    }
    pairs.sort();
    Ok(pairs)
}

/// Every subject/counterpart pair that may lie within `margin_metres` of
/// each other in `projection`, in identity order.
///
/// Each projection prunes by the gap that bounds its own distance from below,
/// so the search stays complete:
///
/// - `Minimum3d`: the Euclidean box gap, as [`candidate_pairs`].
/// - `Horizontal`: the plan box gap. Two bodies on different storeys can be
///   close in plan, so the vertical gap discards nothing.
/// - `PlanOverlap`: footprints that overlap have boxes that meet in plan, so
///   the plan margin is zero whatever `margin_metres` says.
/// - `Vertical`: the bodies are related only when their plan box gap is within
///   the footprint offset, and their distance is at least the vertical gap
///   between their boxes, which must be within the margin. With a direction
///   the gap is one-sided (subject top up to counterpart bottom for `Above`),
///   and a counterpart whose box lies wholly on the other side of the
///   subject's box is certainly not in that direction. A pair is kept when
///   either orientation the groups allow qualifies, since a capability
///   measures a pair reported once from both of its ends.
pub fn projected_candidate_pairs(
    subjects: &[ObjectBounds],
    counterparts: &[ObjectBounds],
    projection: ProximityProjection,
    margin_metres: f64,
) -> Result<Vec<CandidatePair>, CandidateSearchError> {
    if !margin_metres.is_finite() || margin_metres < 0.0 {
        return Err(CandidateSearchError::InvalidMargin);
    }
    let plan_margin = match projection {
        ProximityProjection::Minimum3d => {
            return candidate_pairs(subjects, counterparts, margin_metres);
        }
        ProximityProjection::Horizontal => margin_metres,
        ProximityProjection::PlanOverlap => 0.0,
        ProximityProjection::Vertical {
            footprint_offset_metres,
            ..
        } => footprint_offset_metres,
    };
    // Flattening loses the vertical extent, so refuse conflicting inputs
    // before it could hide a conflict in height.
    let mut enclosing: BTreeMap<&ObjectId, Bounds3> = BTreeMap::new();
    for bounds in subjects.iter().chain(counterparts) {
        let known = enclosing
            .entry(bounds.object())
            .or_insert_with(|| bounds.enclosing());
        if *known != bounds.enclosing() {
            return Err(CandidateSearchError::ConflictingBounds(
                bounds.object().clone(),
            ));
        }
    }
    let flat = |group: &[ObjectBounds]| -> Result<Vec<ObjectBounds>, CandidateSearchError> {
        group
            .iter()
            .map(|bounds| {
                let (min, max) = (bounds.bounds().min(), bounds.bounds().max());
                Bounds3::try_new([min[0], min[1], 0.0], [max[0], max[1], 0.0])
                    .and_then(|flat| {
                        ObjectBounds::try_new(bounds.object().clone(), flat, bounds.fidelity())
                    })
                    .map_err(|_| CandidateSearchError::InvalidMargin)
            })
            .collect()
    };
    let pairs = candidate_pairs(&flat(subjects)?, &flat(counterparts)?, plan_margin)?;
    let ProximityProjection::Vertical { direction, .. } = projection else {
        return Ok(pairs);
    };
    let in_subjects: BTreeSet<&ObjectId> = subjects.iter().map(ObjectBounds::object).collect();
    let in_counterparts: BTreeSet<&ObjectId> =
        counterparts.iter().map(ObjectBounds::object).collect();
    let keep = |subject: &ObjectId, counterpart: &ObjectId, direction| {
        vertically_within(
            &enclosing[subject],
            &enclosing[counterpart],
            direction,
            margin_metres,
        )
    };
    Ok(pairs
        .into_iter()
        .filter(|pair| {
            keep(pair.subject(), pair.counterpart(), direction)
                || (in_subjects.contains(pair.counterpart())
                    && in_counterparts.contains(pair.subject())
                    && keep(pair.counterpart(), pair.subject(), direction))
        })
        .collect())
}

/// Whether a counterpart in `counterpart` may lie within `margin` of a
/// subject in `subject` in `direction`, judged on enclosing boxes.
///
/// The one-sided box gap bounds the one-sided distance from below. A box
/// wholly below the subject's (its top under the subject's bottom) holds a
/// body lower at both ends than the subject, so certainly not above it.
fn vertically_within(
    subject: &Bounds3,
    counterpart: &Bounds3,
    direction: VerticalDirection,
    margin: f64,
) -> bool {
    let rise = counterpart.min()[2] - subject.max()[2];
    let drop = subject.min()[2] - counterpart.max()[2];
    match direction {
        VerticalDirection::Either => rise.max(drop) <= margin,
        VerticalDirection::Above => rise <= margin && drop <= 0.0,
        VerticalDirection::Below => drop <= margin && rise <= 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proximity::GeometryFidelity;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }
    fn unit_box(local: &str, x: f64, fidelity: GeometryFidelity) -> ObjectBounds {
        ObjectBounds::try_new(
            id(local),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0]).unwrap(),
            fidelity,
        )
        .unwrap()
    }
    fn exact(local: &str, x: f64) -> ObjectBounds {
        unit_box(local, x, GeometryFidelity::Exact)
    }
    fn names(pairs: &[CandidatePair]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|p| (p.subject.local_id.clone(), p.counterpart.local_id.clone()))
            .collect()
    }

    /// The broad phase must agree with an exhaustive search. Any pair it drops
    /// is a clash the narrow phase never sees.
    #[test]
    fn sweep_matches_exhaustive_search() {
        let objects: Vec<ObjectBounds> = (0..40)
            .map(|i| {
                let x = f64::from((i * 37) % 23) * 0.7;
                let y = f64::from((i * 11) % 7) * 0.9;
                ObjectBounds::try_new(
                    id(&format!("o{i:02}")),
                    Bounds3::try_new([x, y, 0.0], [x + 1.0, y + 0.5, 1.0]).unwrap(),
                    GeometryFidelity::Exact,
                )
                .unwrap()
            })
            .collect();
        let (subjects, counterparts) = objects.split_at(15);
        for margin in [0.0, 0.3, 2.0] {
            let found = candidate_pairs(subjects, counterparts, margin).unwrap();
            let mut expected = Vec::new();
            for s in subjects {
                for c in counterparts {
                    if s.enclosing().gap(&c.enclosing()) <= margin {
                        expected.push(CandidatePair {
                            subject: s.object().clone(),
                            counterpart: c.object().clone(),
                        });
                    }
                }
            }
            expected.sort();
            assert_eq!(found, expected, "margin {margin}");
        }
    }

    /// The pairs an exhaustive search keeps, each checked on its own.
    fn exhaustive_pairs(
        subjects: &[ObjectBounds],
        counterparts: &[ObjectBounds],
        projection: ProximityProjection,
        margin: f64,
    ) -> Vec<CandidatePair> {
        let gap = |a: &Bounds3, b: &Bounds3, axis: usize| {
            (b.min()[axis] - a.max()[axis])
                .max(a.min()[axis] - b.max()[axis])
                .max(0.0)
        };
        let plan_gap = |a: &Bounds3, b: &Bounds3| gap(a, b, 0).hypot(gap(a, b, 1));
        // Upward gap from the subject's top to the counterpart's bottom.
        let up = |a: &Bounds3, b: &Bounds3| (b.min()[2] - a.max()[2]).max(0.0);
        let wholly_below = |a: &Bounds3, b: &Bounds3| b.max()[2] < a.min()[2];
        let keep = |s: &ObjectBounds, c: &ObjectBounds| {
            let (a, b) = (s.enclosing(), c.enclosing());
            match projection {
                ProximityProjection::Minimum3d => a.gap(&b) <= margin,
                ProximityProjection::Horizontal => plan_gap(&a, &b) <= margin,
                ProximityProjection::PlanOverlap => plan_gap(&a, &b) <= 0.0,
                ProximityProjection::Vertical {
                    footprint_offset_metres,
                    direction,
                    ..
                } => {
                    plan_gap(&a, &b) <= footprint_offset_metres
                        && match direction {
                            VerticalDirection::Either => gap(&a, &b, 2) <= margin,
                            VerticalDirection::Above => {
                                up(&a, &b) <= margin && !wholly_below(&a, &b)
                            }
                            VerticalDirection::Below => {
                                up(&b, &a) <= margin && !wholly_below(&b, &a)
                            }
                        }
                }
            }
        };
        let mut expected = BTreeSet::new();
        for s in subjects {
            for c in counterparts {
                if s.object() == c.object() || !keep(s, c) {
                    continue;
                }
                // Reported once, the lesser identity as subject when the
                // groups allow it.
                let reversible = subjects.iter().any(|o| o.object() == c.object())
                    && counterparts.iter().any(|o| o.object() == s.object());
                let (subject, counterpart) = if reversible && c.object() < s.object() {
                    (c.object(), s.object())
                } else {
                    (s.object(), c.object())
                };
                expected.insert(CandidatePair {
                    subject: subject.clone(),
                    counterpart: counterpart.clone(),
                });
            }
        }
        expected.into_iter().collect()
    }

    /// Each projection must keep every pair whose projected box gap is within
    /// its margin: that gap bounds the projected distance from below. A
    /// vertical direction may drop only pairs whose boxes put the
    /// counterpart certainly on the other side. Disjoint groups keep their
    /// roles; one group checked against itself keeps a pair when either
    /// orientation qualifies.
    #[test]
    fn projected_sweeps_match_exhaustive_search() {
        let objects: Vec<ObjectBounds> = (0..40)
            .map(|i| {
                let x = f64::from((i * 37) % 23) * 0.7;
                let y = f64::from((i * 11) % 7) * 0.9;
                let z = f64::from((i * 5) % 4) * 3.0 + f64::from(i % 3) * 0.2;
                let height = 0.3 + f64::from(i % 5);
                ObjectBounds::try_new(
                    id(&format!("o{i:02}")),
                    Bounds3::try_new([x, y, z], [x + 1.0, y + 0.5, z + height]).unwrap(),
                    if i % 3 == 0 {
                        GeometryFidelity::tessellated(0.05).unwrap()
                    } else {
                        GeometryFidelity::Exact
                    },
                )
                .unwrap()
            })
            .collect();
        let mut projections = vec![
            ProximityProjection::Minimum3d,
            ProximityProjection::Horizontal,
            ProximityProjection::PlanOverlap,
        ];
        for direction in [
            VerticalDirection::Either,
            VerticalDirection::Above,
            VerticalDirection::Below,
        ] {
            for footprint_offset_metres in [0.0, 1.0] {
                projections.push(ProximityProjection::Vertical {
                    footprint_offset_metres,
                    direction,
                    surfaces: crate::VerticalSurfaces::Extents,
                });
            }
        }
        let groups = [
            objects.split_at(15),
            (&objects[..25], &objects[..25]),
            (&objects[10..30], &objects[20..]),
        ];
        for (subjects, counterparts) in groups {
            for margin in [0.0, 0.3, 2.0, 4.0] {
                for &projection in &projections {
                    assert_eq!(
                        projected_candidate_pairs(subjects, counterparts, projection, margin)
                            .unwrap(),
                        exhaustive_pairs(subjects, counterparts, projection, margin),
                        "{projection:?} margin {margin}"
                    );
                }
            }
        }
    }

    /// A sprinkler under a ceiling and over a floor: each direction keeps
    /// only its own side, within the margin.
    #[test]
    fn a_vertical_direction_keeps_only_its_side() {
        let body = |local: &str, z: [f64; 2]| {
            ObjectBounds::try_new(
                id(local),
                Bounds3::try_new([0.0, 0.0, z[0]], [1.0, 1.0, z[1]]).unwrap(),
                GeometryFidelity::Exact,
            )
            .unwrap()
        };
        let sprinkler = body("sprinkler", [2.6, 2.7]);
        let others = [
            body("ceiling", [3.0, 3.2]),
            body("floor", [-0.2, 0.0]),
            body("riser", [-1.0, 4.0]),
        ];
        let kept = |direction, margin| {
            names(
                &projected_candidate_pairs(
                    std::slice::from_ref(&sprinkler),
                    &others,
                    ProximityProjection::Vertical {
                        footprint_offset_metres: 0.0,
                        direction,
                        surfaces: crate::VerticalSurfaces::Extents,
                    },
                    margin,
                )
                .unwrap(),
            )
            .into_iter()
            .map(|(_, counterpart)| counterpart)
            .collect::<Vec<_>>()
        };
        assert_eq!(kept(VerticalDirection::Above, 0.5), ["ceiling", "riser"]);
        assert_eq!(kept(VerticalDirection::Below, 0.5), ["riser"]);
        assert_eq!(kept(VerticalDirection::Below, 3.0), ["floor", "riser"]);
        assert_eq!(kept(VerticalDirection::Either, 0.5), ["ceiling", "riser"]);
    }

    /// Bodies on different storeys are close in plan and above one another.
    #[test]
    fn projections_ignore_the_height_that_does_not_measure_them() {
        let low = exact("low", 0.0);
        let high = ObjectBounds::try_new(
            id("high"),
            Bounds3::try_new([0.5, 0.0, 10.0], [1.5, 1.0, 11.0]).unwrap(),
            GeometryFidelity::Exact,
        )
        .unwrap();
        let (subjects, counterparts) = (&[low][..], &[high][..]);
        assert!(
            candidate_pairs(subjects, counterparts, 1.0)
                .unwrap()
                .is_empty()
        );
        for projection in [
            ProximityProjection::Horizontal,
            ProximityProjection::PlanOverlap,
        ] {
            assert_eq!(
                projected_candidate_pairs(subjects, counterparts, projection, 0.0)
                    .unwrap()
                    .len(),
                1
            );
        }
        let vertical = ProximityProjection::Vertical {
            footprint_offset_metres: 0.0,
            direction: VerticalDirection::Either,
            surfaces: crate::VerticalSurfaces::Extents,
        };
        assert_eq!(
            projected_candidate_pairs(subjects, counterparts, vertical, 9.0)
                .unwrap()
                .len(),
            1
        );
        assert!(
            projected_candidate_pairs(subjects, counterparts, vertical, 8.5)
                .unwrap()
                .is_empty()
        );
        let conflicting = ObjectBounds::try_new(
            id("low"),
            Bounds3::try_new([0.0, 0.0, 5.0], [1.0, 1.0, 6.0]).unwrap(),
            GeometryFidelity::Exact,
        )
        .unwrap();
        assert_eq!(
            projected_candidate_pairs(
                subjects,
                &[conflicting],
                ProximityProjection::Horizontal,
                0.0
            ),
            Err(CandidateSearchError::ConflictingBounds(id("low")))
        );
    }

    #[test]
    fn a_symmetric_search_reports_each_pair_once_and_never_self() {
        let all = [exact("a", 0.0), exact("b", 0.5), exact("c", 5.0)];
        let pairs = candidate_pairs(&all, &all, 0.0).unwrap();
        assert_eq!(names(&pairs), vec![("a".into(), "b".into())]);
    }

    #[test]
    fn subject_and_counterpart_roles_are_kept() {
        let pairs = candidate_pairs(&[exact("z", 0.0)], &[exact("a", 0.5)], 0.0).unwrap();
        assert_eq!(names(&pairs), vec![("z".into(), "a".into())]);
    }

    /// A tessellated cylinder's true surface extends past its mesh. The pair
    /// must survive even though the mesh boxes are apart.
    #[test]
    fn tessellation_deviation_widens_the_search() {
        let pipe = unit_box("pipe", 0.0, GeometryFidelity::tessellated(0.01).unwrap());
        let wall = exact("wall", 1.005);
        assert_eq!(candidate_pairs(&[pipe], &[wall], 0.0).unwrap().len(), 1);
    }

    #[test]
    fn conflicting_bounds_and_bad_margins_are_refused() {
        assert_eq!(
            candidate_pairs(&[exact("a", 0.0)], &[exact("a", 3.0)], 0.0),
            Err(CandidateSearchError::ConflictingBounds(id("a")))
        );
        for margin in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                candidate_pairs(&[], &[], margin),
                Err(CandidateSearchError::InvalidMargin)
            );
        }
    }
}
