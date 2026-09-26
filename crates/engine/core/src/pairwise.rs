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

use std::collections::BTreeMap;

use axioval_ir::ObjectId;

use crate::proximity::{Bounds3, ObjectBounds};

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
