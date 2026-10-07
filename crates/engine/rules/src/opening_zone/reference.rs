//! The implementation `opening-zone`'s template replaced, kept to hold the
//! template to on generated inputs (`parity-reference` only).

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ParameterDescriptor,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use super::dimensions::{Applied, Verdict};
use super::face::{Host, ROUNDING, Span};
use super::supports::{Decided, Opening, Supports};
use super::{Clearance, Config, Judge, Placed, list};
use crate::counts::Population;
use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::support::{Unavailable, finding};

/// `opening-zone` as it was evaluated before it ran as a template.
pub struct OpeningZone;

/// A finding's message and, where it misses a bound, by how far.
type Found = (String, Option<Deviation>);

impl RuleCapability for OpeningZone {
    fn id(&self) -> &'static str {
        super::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("opening-zone: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let openings = Population::of(context, &rule.selector);
        let hosts = Population::of(context, config.host_selector);
        let support_population = config
            .supports
            .as_ref()
            .map(|supports| Population::of(context, supports.selector));
        let dimension_selections = super::dimensions::Selections::of(context, &config.dimensions);
        let mut judge = Judge {
            dimensions: dimension_selections,
            context,
            rule,
            config: &config,
            hosts: &hosts,
            bodies: BTreeMap::new(),
            placed: BTreeMap::new(),
            supports: support_population
                .as_ref()
                .zip(config.supports.as_ref())
                .map(|(population, config)| Supports::new(context, config, population)),
        };
        // Every opening that may be selected is placed, so spacing sees the
        // undecided ones too.
        let candidates: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| openings.contains(&object.id))
            .collect();
        for opening in &candidates {
            let placed = judge.place(opening);
            judge.placed.insert(opening.id.clone(), placed);
        }
        for opening in selected {
            judge.judge(opening, &openings, &mut evaluation);
        }
        evaluation
    }
}

impl Judge<'_, '_> {
    fn judge(
        &self,
        opening: &Object,
        openings: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let placements = match self.placed.get(&opening.id) {
            Some(Ok(placements)) => placements,
            Some(Err((reason, message))) => {
                evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                return;
            }
            None => {
                evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "the opening was not placed".to_owned(),
                );
                return;
            }
        };
        for placed in placements {
            match placed {
                Ok(placed) if placed.may_be_small => evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "whether its area in its host {} is below `minimum_opening_area` is \
                         undecided: it is not extruded through the host",
                        placed.host.local_id
                    ),
                ),
                Ok(placed) => self.judge_in(opening, placed, openings, evaluation),
                Err((reason, message)) => evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    reason.clone(),
                    message.clone(),
                ),
            }
        }
    }

    /// Judges an opening placed in one of its hosts.
    fn judge_in(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) {
        // Placing the opening read its host.
        let Some(Ok(host)) = self.bodies.get(&placed.host).cloned() else {
            return;
        };
        let mut findings = Vec::new();
        let (_, length_bounds) = host.axis(self.config.axes.length);
        let (_, height_bounds) = host.axis(self.config.axes.height);
        let (outside_length, outside_height) = self.beyond(&host, placed);
        let outside: Vec<String> = [
            ("length", outside_length, placed.length, length_bounds),
            ("height", outside_height, placed.height, height_bounds),
        ]
        .into_iter()
        .filter(|(_, out, _, _)| *out)
        .map(|(name, _, extent, bounds)| {
            format!(
                "along its {name} it spans {} to {}, the host {} to {}",
                metres(extent.0),
                metres(extent.1),
                metres(bounds.0),
                metres(bounds.1)
            )
        })
        .collect();
        let axes = self.config.axes;
        let rect = placed.section_rect(&host, axes);
        let mut crosses_outline = false;
        if outside.is_empty() {
            crosses_outline = Self::crosses_outline(&host, placed, rect, &mut findings)
                .unwrap_or_else(|(reason, message)| {
                    evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                    true
                });
        } else {
            findings.push((
                format!(
                    "opening lies partly outside its host {}: {}",
                    placed.host.local_id,
                    outside.join("; ")
                ),
                None,
            ));
        }
        if !outside_length
            && !crosses_outline
            && let Err((reason, message)) = self.ends(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_height
            && !crosses_outline
            && let Err((reason, message)) = self.edges(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_height
            && !crosses_outline
            && let Err((reason, message)) = self.far_edges(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_length && !outside_height && !crosses_outline {
            match self.zone_miss(&host, placed, rect) {
                Ok(None) => {}
                Ok(Some(miss)) => findings.push((miss.message, Some(miss.deviation))),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                }
            }
        }
        if !outside_length && !outside_height && !crosses_outline {
            self.dimensions(opening, placed, &host, rect, evaluation);
        }
        if let Some(supports) = &self.supports {
            let face = Opening {
                host: &placed.host,
                length: placed.length,
                height: placed.height,
                exact: placed.exact,
            };
            for decided in supports.judge(&face, &host, self.config.axes) {
                match decided {
                    Decided::Pass => {}
                    Decided::Unfound((reason, message)) => {
                        evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                    }
                    Decided::Open(message) => evaluation.push_object_not_evaluated(
                        opening.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        message,
                    ),
                    Decided::Finding((message, evidence, related)) => {
                        let mut cited = placed.evidence.clone();
                        cited.extend(evidence);
                        evaluation
                            .push_finding(self.finding(opening, placed, message, &cited, &related));
                    }
                }
            }
        }
        self.spacing(opening, placed, openings, findings, evaluation);
    }

    /// Within the box that holds a free outline, whether the opening
    /// crosses the outline itself; an error when it may.
    fn crosses_outline(
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<bool, Unavailable> {
        let Some(crosses) = Self::crossing(host, placed, rect) else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it may cross the edge of its host {}'s outline: it is not extruded \
                     straight through the host, so where it lies in the host's section is \
                     known only within bounds",
                    placed.host.local_id
                ),
            ));
        };
        if !crosses {
            return Ok(false);
        }
        findings.push((
            format!(
                "opening lies partly outside its host {}: it crosses the edge of the host's \
                 outline",
                placed.host.local_id
            ),
            None,
        ));
        Ok(true)
    }

    /// Judges the distance from the host's ends.
    fn ends(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        let Some(required) = self.config.end_distance else {
            return Ok(());
        };
        let Some((clear, exact)) = self.end_clearance(host, placed, rect) else {
            return Ok(());
        };
        if clear >= required - ROUNDING {
            return Ok(());
        }
        if !exact {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "its distance from an end of its host {}'s outline may be under {}: where \
                     it lies in the host's section is known only within bounds",
                    placed.host.local_id,
                    metres(required)
                ),
            ));
        }
        findings.push((
            format!(
                "opening is {} from an end of its host {}; {} required",
                metres(clear.max(0.0)),
                placed.host.local_id,
                metres(required)
            ),
            Some(Deviation::below(required, clear, clear)),
        ));
        Ok(())
    }

    /// Judges the clearance from the host's edges, or from its flanges.
    fn edges(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        if self.config.edge_distance.is_none() && !self.config.web {
            return Ok(());
        }
        let required = self.config.edge_distance.unwrap_or(0.0);
        let Some(Clearance {
            clear,
            exact,
            outline,
            what,
        }) = self.edge_clearance(host, placed, rect)?
        else {
            return Ok(());
        };
        if clear >= required - ROUNDING {
            return Ok(());
        }
        if outline {
            if !exact {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "its distance from an edge of its host {}'s outline may be under {}: \
                         where it lies in the host's section is known only within bounds",
                        placed.host.local_id,
                        metres(required)
                    ),
                ));
            }
            findings.push((
                format!(
                    "opening is {} from an edge of its host {}; {} clear required",
                    metres(clear.max(0.0)),
                    placed.host.local_id,
                    metres(required)
                ),
                Some(Deviation::below(required, clear, clear)),
            ));
            return Ok(());
        }
        let distance = if clear < 0.0 {
            format!("reaches {} into", metres(-clear))
        } else {
            format!("is {} from", metres(clear))
        };
        findings.push((
            format!(
                "opening {distance} {what} of its host {}; {} clear required",
                placed.host.local_id,
                metres(required)
            ),
            Some(Deviation::below(required, clear, clear)),
        ));
        Ok(())
    }

    /// Judges the largest distance allowed from the host's low and high
    /// edges (or flanges, with `zone` `web`). A distance measured to a free
    /// outline from the box the opening may lie in is a lower bound, which can
    /// find an opening too far but never pass one.
    fn far_edges(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        let Some((maximum, low, high)) = self.config.edge_maximum else {
            return Ok(());
        };
        let Some((clear, exact)) = self.far_clearance(host, placed, rect)? else {
            return Ok(());
        };
        let what = if self.config.web {
            ["the lower flange", "the upper flange"]
        } else {
            ["the bottom edge", "the top edge"]
        };
        let mut open = Vec::new();
        for (checked, distance, edge) in [(low, clear.0, what[0]), (high, clear.1, what[1])] {
            if !checked {
                continue;
            }
            // The distance is exact, or a lower bound of the true one.
            if distance > maximum + ROUNDING {
                // A lower bound over the maximum may be farther still.
                let upper = if exact { distance } else { f64::INFINITY };
                findings.push((
                    format!(
                        "opening is {} from {edge} of its host {}; at most {} allowed",
                        metres(distance),
                        placed.host.local_id,
                        metres(maximum)
                    ),
                    Some(Deviation::above(maximum, distance, upper)),
                ));
            } else if !exact {
                open.push(edge);
            }
        }
        if open.is_empty() {
            return Ok(());
        }
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "its distance from {} of its host {}'s outline may exceed {}: where it lies \
                 in the host's section is known only within bounds",
                open.join(" and "),
                placed.host.local_id,
                metres(maximum)
            ),
        ))
    }

    /// Judges every row of the dimensioning table whose `source` selects
    /// the opening.
    fn dimensions(
        &self,
        opening: &Object,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        evaluation: &mut CapabilityEvaluation,
    ) {
        for applied in self.applied(opening, placed, host, rect) {
            let (index, measure) = match applied {
                Applied::Undecided(message) => {
                    evaluation.push_object_not_evaluated(
                        opening.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        message,
                    );
                    continue;
                }
                Applied::Measured(_, Err((reason, message))) => {
                    evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                    continue;
                }
                Applied::Measured(index, Ok(measure)) => (index, measure),
            };
            let dimension = &self.config.dimensions[index];
            match dimension.bound.judge(measure.distance, dimension.tolerance) {
                Verdict::Holds => {}
                Verdict::Misses(required, deviation) => {
                    let mut evidence = placed.evidence.clone();
                    let mut related = Vec::new();
                    if let Some((target, neighbour)) = measure.target {
                        evidence.extend(neighbour.evidence.iter().cloned());
                        related.push(target.clone());
                    }
                    evaluation.push_graded_finding(
                        self.finding(
                            opening,
                            placed,
                            format!("{}; {required} ({})", measure.words, dimension.label),
                            &evidence,
                            &related,
                        ),
                        deviation,
                    );
                }
                Verdict::Undecided => evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    measure.open,
                ),
            }
        }
    }

    /// Judges the clear distance to the other openings of the same host and
    /// records every finding of `opening`.
    fn spacing(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        mut findings: Vec<Found>,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let mut evidence = placed.evidence.clone();
        let mut related = Vec::new();
        if let Some(required) = self.config.spacing {
            let spacing = self.spacing_of(opening, placed, openings, required);
            if spacing.sure.is_empty() {
                if !spacing.unknown.is_empty() {
                    evaluation.push_object_not_evaluated(
                        opening.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "its clear distance to {} may be under {}: their outlines or \
                             hosts are not known exactly",
                            list(&spacing.unknown),
                            metres(required)
                        ),
                    );
                }
            } else {
                let nearest = spacing.nearest();
                findings.push((
                    format!(
                        "opening is {} clear of another opening in its host {}; {} required",
                        metres(nearest),
                        placed.host.local_id,
                        metres(required)
                    ),
                    Some(Deviation::below(required, nearest, nearest)),
                ));
                for (other, _, neighbour) in spacing.sure {
                    evidence.extend(neighbour.evidence.iter().cloned());
                    related.push(other);
                }
            }
        }
        for (message, deviation) in findings {
            evaluation.push_finding_deviating(
                self.finding(opening, placed, message, &evidence, &related),
                deviation,
            );
        }
    }

    fn finding(
        &self,
        opening: &Object,
        placed: &Placed,
        message: String,
        evidence: &[Evidence],
        related: &[ObjectId],
    ) -> Finding {
        let mut related = related.to_vec();
        related.push(placed.host.clone());
        finding(self.rule, &opening.id, message, evidence.to_vec(), related)
    }
}
