//! Special-case tolerances along the elements' own axes.
//!
//! World axes misjudge elements at an angle: a slab edge sunk 10 mm into a
//! wall at 30° reaches far along x and y, so no world-axis tolerance lets it
//! pass. A `tolerance_cases` row names a case, two component filters and a
//! tolerance, and excuses an intersection between a pair matching the
//! filters (`first_selector` on one member, `second_selector` on the other,
//! either way round) whose extent along the elements' own placement axes
//! stays within the tolerance:
//!
//! | Case | Measured along |
//! |---|---|
//! | `horizontal_orthogonal` | the second element's plan axes, the lesser extent |
//! | `vertical_orthogonal` | the second element's vertical axis |
//! | `horizontal_protrusion` | the first element's plan axes, the lesser extent |
//! | `vertical_protrusion` | the first element's vertical axis |
//!
//! An orthogonal case measures how far the first element reaches into the
//! second, across the second's own axes (a slab edge into a wall's
//! thickness); a protrusion how far the first element sticks out, along its
//! own. The axes are the placement frames the object-frame service states,
//! and the extents come from the proximity service
//! ([`axioval_engine::OverlapAlongRequest`]).
//!
//! Everything is three-valued. A case excuses a pair only when its filters
//! surely match and the whole measured extent lies within the tolerance; it
//! leaves the pair open when a filter or the extent is undecided, or when a
//! frame or the extents cannot be read.

use std::collections::BTreeMap;

use axioval_engine::{
    ColumnKind, LengthInterval, MetricDirection, ObjectFrameServiceHandle, OverlapAlongRequest,
    ParameterDescriptor, ParameterType, ProximityServiceHandle, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use crate::clash::Holds;
use crate::pairs::reason;
use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, Unavailable, invalid};

const CASE_COLUMNS: &[TableColumn] = &[
    TableColumn::required("case", ColumnKind::String),
    TableColumn::optional("first_selector", ColumnKind::Selector),
    TableColumn::optional("second_selector", ColumnKind::Selector),
    TableColumn::required("tolerance_metres", ColumnKind::Number),
];

/// The `tolerance_cases` parameter `clash` and `clash-matrix` share.
pub(crate) fn case_parameter() -> ParameterDescriptor {
    ParameterDescriptor::optional("tolerance_cases", ParameterType::Table(CASE_COLUMNS))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    HorizontalOrthogonal,
    VerticalOrthogonal,
    HorizontalProtrusion,
    VerticalProtrusion,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::HorizontalOrthogonal => "horizontal orthogonal",
            Self::VerticalOrthogonal => "vertical orthogonal",
            Self::HorizontalProtrusion => "horizontal protrusion",
            Self::VerticalProtrusion => "vertical protrusion",
        }
    }
    /// Whether the extent is measured along the second element's axes.
    fn along_second(self) -> bool {
        matches!(self, Self::HorizontalOrthogonal | Self::VerticalOrthogonal)
    }
    fn horizontal(self) -> bool {
        matches!(
            self,
            Self::HorizontalOrthogonal | Self::HorizontalProtrusion
        )
    }
}

struct Case<'a> {
    kind: Kind,
    /// The filters on the first and the second element.
    selectors: [Option<&'a Selector>; 2],
    tolerance: f64,
}

/// The declared tolerance cases.
pub(crate) struct Cases<'a>(Vec<Case<'a>>);

/// Reads `tolerance_cases`; no rows declare no case.
pub(crate) fn cases<'a>(parameters: &Parameters<'a>) -> Result<Cases<'a>, Unavailable> {
    let mut cases = Vec::new();
    for (index, row) in parameters
        .table("tolerance_cases")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let kind = match row.text("case")?.unwrap_or_default() {
            "horizontal_orthogonal" => Kind::HorizontalOrthogonal,
            "vertical_orthogonal" => Kind::VerticalOrthogonal,
            "horizontal_protrusion" => Kind::HorizontalProtrusion,
            "vertical_protrusion" => Kind::VerticalProtrusion,
            other => {
                return Err(invalid(format!(
                    "tolerance case {index}: `{other}` is not `horizontal_orthogonal`, \
                     `vertical_orthogonal`, `horizontal_protrusion` or `vertical_protrusion`"
                )));
            }
        };
        let tolerance = row.number("tolerance_metres")?.unwrap_or(-1.0);
        if !(tolerance >= 0.0 && tolerance.is_finite()) {
            return Err(invalid(format!(
                "tolerance case {index}: `tolerance_metres` must not be negative"
            )));
        }
        cases.push(Case {
            kind,
            selectors: [
                row.selector("first_selector")?,
                row.selector("second_selector")?,
            ],
            tolerance,
        });
    }
    Ok(Cases(cases))
}

impl Cases<'_> {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Whether a declared case excuses a pair's intersection, why not when it
/// cannot be told, and the evidence read.
pub(crate) struct Excuse {
    pub(crate) holds: Holds,
    /// Why a case may or may not excuse the pair, for an open outcome.
    pub(crate) note: String,
    pub(crate) evidence: Vec<Evidence>,
}

impl Excuse {
    pub(crate) fn none() -> Self {
        Self {
            holds: Holds::No,
            note: String::new(),
            evidence: Vec::new(),
        }
    }
}

type Axes = [MetricDirection; 3];

/// Judges the declared cases against pairs, reading each frame and
/// selection once.
pub(crate) struct CaseJudge<'r> {
    context: &'r RuleContext<'r>,
    cases: &'r Cases<'r>,
    frames: BTreeMap<ObjectId, Result<(Axes, Evidence), Unavailable>>,
    selections: BTreeMap<(usize, usize, ObjectId), Holds>,
}

impl<'r> CaseJudge<'r> {
    pub(crate) fn new(context: &'r RuleContext<'r>, cases: &'r Cases<'r>) -> Self {
        Self {
            context,
            cases,
            frames: BTreeMap::new(),
            selections: BTreeMap::new(),
        }
    }

    fn selected(&mut self, case: usize, side: usize, object: &ObjectId) -> Holds {
        let context = self.context;
        let Some(selector) = self.cases.0[case].selectors[side] else {
            return Holds::Yes;
        };
        *self
            .selections
            .entry((case, side, object.clone()))
            .or_insert_with(|| {
                let Some(found) = context.project.object(object) else {
                    return Holds::Unknown;
                };
                match selector_matches(context, selector, found, &mut Vec::new()) {
                    Selection::Match => Holds::Yes,
                    Selection::NoMatch => Holds::No,
                    Selection::NotEvaluated(..) => Holds::Unknown,
                }
            })
    }

    fn frame(&mut self, object: &ObjectId) -> Result<(Axes, Evidence), Unavailable> {
        let context = self.context;
        self.frames
            .entry(object.clone())
            .or_insert_with(|| {
                let service = context
                    .services
                    .get::<ObjectFrameServiceHandle>()
                    .ok_or_else(|| {
                        (
                            axioval_ir::NotEvaluatedReason::MissingService,
                            "object-frame service is not registered".to_owned(),
                        )
                    })?;
                let placed = service.object_frame(object).map_err(|error| {
                    (
                        axioval_ir::NotEvaluatedReason::IncompleteEvidence,
                        format!("the axes of {object} cannot be read: {error}"),
                    )
                })?;
                let frame = placed.frame();
                Ok((
                    [frame.right(), frame.forward(), frame.up()],
                    placed.evidence().clone(),
                ))
            })
            .clone()
    }

    /// Whether a case excuses the intersection of `subject` and
    /// `counterpart`: yes when one surely does, no when none may.
    pub(crate) fn excuses(
        &mut self,
        service: &ProximityServiceHandle,
        subject: &ObjectId,
        counterpart: &ObjectId,
    ) -> Excuse {
        // Each case that may apply, with its first and second element.
        let mut applying = Vec::new();
        for case in 0..self.cases.0.len() {
            for (first, second) in [(subject, counterpart), (counterpart, subject)] {
                let matched = self
                    .selected(case, 0, first)
                    .and(self.selected(case, 1, second));
                if matched != Holds::No {
                    applying.push((case, first, matched));
                }
            }
        }
        if applying.is_empty() {
            return Excuse::none();
        }
        let measured = self.measure(service, subject, counterpart);
        let mut excused = Holds::No;
        let mut notes = Vec::new();
        let mut evidence = Vec::new();
        for (case, first, matched) in applying {
            let declared = &self.cases.0[case];
            let (holds, note) = match &measured {
                Ok((extents, _)) => {
                    let first_is_subject = first == subject;
                    let body_is_subject = first_is_subject != declared.kind.along_second();
                    let axes = if body_is_subject {
                        &extents[..3]
                    } else {
                        &extents[3..]
                    };
                    let extent = if declared.kind.horizontal() {
                        lesser(axes[0], axes[1])
                    } else {
                        axes[2]
                    };
                    let body = if body_is_subject {
                        subject
                    } else {
                        counterpart
                    };
                    let holds = if extent.upper_metres() <= declared.tolerance {
                        Holds::Yes
                    } else if extent.lower_metres() > declared.tolerance {
                        Holds::No
                    } else {
                        Holds::Unknown
                    };
                    (
                        holds,
                        format!(
                            "{} case {case} ({:.4} m): {} along the axes of {body}",
                            declared.kind.name(),
                            declared.tolerance,
                            described(extent)
                        ),
                    )
                }
                Err((_, message)) => (
                    Holds::Unknown,
                    format!(
                        "{} case {case} ({:.4} m): {message}",
                        declared.kind.name(),
                        declared.tolerance
                    ),
                ),
            };
            let holds = matched.and(holds);
            if holds != Holds::No {
                notes.push(note);
            }
            excused = excused.or(holds);
        }
        if let Ok((_, read)) = measured {
            evidence = read;
        }
        Excuse {
            holds: excused,
            note: if notes.is_empty() {
                String::new()
            } else {
                format!(
                    ", or whether a tolerance case excuses it: {}",
                    notes.join("; ")
                )
            },
            evidence,
        }
    }

    /// The pair's extents along the subject's three axes, then the
    /// counterpart's, with the evidence of the frames and the measurement.
    fn measure(
        &mut self,
        service: &ProximityServiceHandle,
        subject: &ObjectId,
        counterpart: &ObjectId,
    ) -> Result<(Vec<LengthInterval>, Vec<Evidence>), Unavailable> {
        let (subject_axes, subject_frame) = self.frame(subject)?;
        let (counterpart_axes, counterpart_frame) = self.frame(counterpart)?;
        let directions: Vec<MetricDirection> =
            subject_axes.into_iter().chain(counterpart_axes).collect();
        let measured =
            OverlapAlongRequest::try_new(subject.clone(), counterpart.clone(), directions)
                .and_then(|request| service.measure_overlap_along(&request))
                .map_err(|error| {
                    (
                        reason(error),
                        format!(
                            "its extents along the elements' axes could not be measured: {error}"
                        ),
                    )
                })?;
        Ok((
            measured.extents().to_vec(),
            vec![
                subject_frame,
                counterpart_frame,
                measured.evidence().clone(),
            ],
        ))
    }
}

/// The lesser of two extents, bound by bound.
fn lesser(a: LengthInterval, b: LengthInterval) -> LengthInterval {
    LengthInterval::try_new(
        a.lower_metres().min(b.lower_metres()),
        a.upper_metres().min(b.upper_metres()),
    )
    .unwrap_or_else(|_| unreachable!("the lesser of two intervals is an interval"))
}

fn described(extent: LengthInterval) -> String {
    if extent.is_exact() {
        format!("{:.4} m", extent.lower_metres())
    } else {
        format!(
            "{:.4} to {:.4} m",
            extent.lower_metres(),
            extent.upper_metres()
        )
    }
}
