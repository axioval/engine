//! The implementations `clash` and `clash-matrix` replaced, kept as the
//! parity references their templates are held to (`parity-reference`):
//! each pair judged by its profile's classes in code, graded, grouped and
//! recorded as the capabilities did. The measurement, the exclusions, the
//! cells, the tolerance cases and the group keys are shared with the
//! measured lists, so both sides measure alike.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ProximityEvidence,
    ProximityProjection, ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, ObjectId, Scope, Severity};

use super::measured::CaseJudge;
use super::{
    Class, Exclusions, Holds, Profile, declaration, described, described_volume, excluded_words,
    measure,
};
use crate::clash_cases::{Cases, Excuse};
use crate::clash_groups::{Context, GroupBy, GroupKeys, Grouping, Key};
use crate::clash_matrix::Categories;
use crate::clash_severity::{Severities, label};
use crate::pairs::{Unevaluated, fidelity_note, prepare, refuse_declaration, severity};
use crate::support::Unavailable;
use crate::support::table::{Matched, RowSelection, match_rows};

/// Whether a tolerance case excuses a pair's intersection, asked only when
/// it can change the outcome, and why that is open when it is.
type Excused<'a> = &'a mut dyn FnMut() -> (Holds, String);

/// What a pair amounts to.
pub(crate) enum Outcome {
    Pass,
    Finding(Class, String),
    Open(NotEvaluatedReason, String),
}

impl Outcome {
    /// The outcome when the measurement cannot tell `yes` from `no`: it
    /// stands only where both agree. Two findings keep the one that holds
    /// either way, `no`.
    fn either(yes: Self, no: Self, undecided: String) -> Self {
        match (yes, no) {
            (Self::Pass, Self::Pass) => Self::Pass,
            (Self::Finding(..), Self::Finding(class, message)) => Self::Finding(class, message),
            (_, Self::Open(reason, message)) | (Self::Open(reason, message), _) => {
                Self::Open(reason, message)
            }
            _ => Self::Open(NotEvaluatedReason::IncompleteEvidence, undecided),
        }
    }
}

impl Profile {
    fn judge(
        &self,
        measured: &ProximityEvidence,
        counterpart: &ObjectId,
        excused: Excused<'_>,
    ) -> Outcome {
        let note = fidelity_note(measured.fidelity());
        let tolerance = self.duplicate_tolerance;
        let (lower, upper) = match measured.hausdorff_interval_metres() {
            Some(interval) => (interval.lower_metres(), interval.upper_metres()),
            None => (0.0, f64::INFINITY),
        };
        // No point of a surface lies nearer the other than the separation.
        let lower = lower.max(measured.separation_interval_metres().0);
        let duplicate = if upper <= tolerance {
            Holds::Yes
        } else if lower > tolerance {
            Holds::No
        } else {
            Holds::Unknown
        };
        let reported = || {
            if self.report.duplicate {
                Outcome::Finding(
                    Class::Duplicate,
                    format!(
                        "duplicate of {counterpart}: the surfaces lie within {upper:.4} m of each other, tolerance {tolerance:.4} m{note}"
                    ),
                )
            } else {
                Outcome::Pass
            }
        };
        match duplicate {
            Holds::Yes => reported(),
            Holds::No => self.distinct(measured, counterpart, &note, excused),
            Holds::Unknown => Outcome::either(
                reported(),
                self.distinct(measured, counterpart, &note, excused),
                format!(
                    "whether {counterpart} is a duplicate cannot be decided: the surfaces lie between {lower:.4} m and {} of each other, tolerance {tolerance:.4} m{note}",
                    if upper.is_finite() {
                        format!("{upper:.4} m")
                    } else {
                        "an unmeasured distance".to_owned()
                    }
                ),
            ),
        }
    }

    /// The outcome for a pair that is not a duplicate.
    fn distinct(
        &self,
        measured: &ProximityEvidence,
        counterpart: &ObjectId,
        note: &str,
        excused: Excused<'_>,
    ) -> Outcome {
        use axioval_engine::BodyContainment;
        let containment = |message: String| {
            if self.report.containment {
                Outcome::Finding(Class::Containment, message)
            } else {
                Outcome::Pass
            }
        };
        match (measured.containment(), measured.penetration_metres()) {
            (Some(BodyContainment::SubjectInsideCounterpart), _) => {
                containment(format!("lies wholly inside {counterpart}{note}"))
            }
            (Some(BodyContainment::CounterpartInsideSubject), _) => {
                containment(format!("wholly contains {counterpart}{note}"))
            }
            (None, Some(depth)) if depth > self.penetration_tolerance => {
                self.intersection(measured, counterpart, (depth, note), excused)
            }
            (None, None) if measured.separation_metres() == 0.0 => Outcome::Open(
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "surfaces meet {counterpart}, but neither body is a closed solid, so touching cannot be told from crossing"
                ),
            ),
            (None, _) => self.clearance(measured, counterpart, note),
        }
    }

    /// A penetration past the tolerance: an intersection when it also
    /// reaches past the axis tolerances, shares more than the volume
    /// tolerance, and no tolerance case excuses it.
    fn intersection(
        &self,
        measured: &ProximityEvidence,
        counterpart: &ObjectId,
        (depth, note): (f64, &str),
        excused: Excused<'_>,
    ) -> Outcome {
        let extents = measured.overlap_extents();
        let exceeds = |tolerance: f64, interval: Option<axioval_engine::LengthInterval>| {
            if tolerance == 0.0 {
                return Holds::Yes;
            }
            match interval {
                Some(interval) if interval.lower_metres() > tolerance => Holds::Yes,
                Some(interval) if interval.upper_metres() <= tolerance => Holds::No,
                _ => Holds::Unknown,
            }
        };
        let holds = exceeds(
            self.horizontal_tolerance,
            extents.map(|extents| extents.horizontal()),
        )
        .and(exceeds(
            self.vertical_tolerance,
            extents.map(|extents| extents.vertical()),
        ));
        let shared = measured.intersection_volume().map(|volume| volume.shared());
        let holds = holds.and(if self.volume_tolerance == 0.0 {
            Holds::Yes
        } else {
            match shared {
                Some(shared) if shared.lower_cubic_metres() > self.volume_tolerance => Holds::Yes,
                Some(shared) if shared.upper_cubic_metres() <= self.volume_tolerance => Holds::No,
                _ => Holds::Unknown,
            }
        });
        // A case can only excuse what is reported and not already passed.
        let (holds, case) = if holds == Holds::No || !self.report.intersection {
            (holds, String::new())
        } else {
            let (excuse, case) = excused();
            (holds.and(excuse.not()), case)
        };
        let axes = self.horizontal_tolerance > 0.0 || self.vertical_tolerance > 0.0;
        let reach = if axes {
            format!(
                ", reaching {} in plan and {} vertically",
                described(extents.map(|extents| extents.horizontal())),
                described(extents.map(|extents| extents.vertical()))
            )
        } else {
            String::new()
        };
        let reach = if self.volume_tolerance > 0.0 {
            format!("{reach}, sharing {}", described_volume(shared))
        } else {
            reach
        };
        let reported = || {
            if self.report.intersection {
                Outcome::Finding(
                    Class::Intersection,
                    format!(
                        "hard clash with {counterpart}: penetration {depth:.4} m exceeds tolerance {:.4} m{reach}{note}",
                        self.penetration_tolerance
                    ),
                )
            } else {
                Outcome::Pass
            }
        };
        match holds {
            Holds::Yes => reported(),
            Holds::No => self.clearance(measured, counterpart, note),
            Holds::Unknown => Outcome::either(
                reported(),
                self.clearance(measured, counterpart, note),
                format!(
                    "whether the intersection with {counterpart} exceeds the horizontal tolerance {:.4} m, the vertical tolerance {:.4} m and the volume tolerance {:.6} m³{case} cannot be decided{reach}{note}",
                    self.horizontal_tolerance, self.vertical_tolerance, self.volume_tolerance
                ),
            ),
        }
    }

    fn clearance(
        &self,
        measured: &ProximityEvidence,
        counterpart: &ObjectId,
        note: &str,
    ) -> Outcome {
        // A certified separation is judged as an interval: below the
        // clearance only when all of it is, open when it straddles.
        if let (Some(clearance), Some(certified)) =
            (self.clearance, measured.certified_separation())
        {
            let (lower, upper) = (certified.lower_metres(), certified.upper_metres());
            return if upper < clearance {
                Outcome::Finding(
                    Class::Clearance,
                    format!(
                        "clearance clash with {counterpart}: separation certified within [{lower:.6}, {upper:.6}] m, below required {clearance:.4} m{note}"
                    ),
                )
            } else if lower >= clearance {
                Outcome::Pass
            } else {
                Outcome::Open(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "whether {counterpart} keeps the clearance {clearance:.4} m cannot be decided: separation certified within [{lower:.6}, {upper:.6}] m{note}"
                    ),
                )
            };
        }
        match self.clearance {
            Some(clearance) if measured.separation_metres() < clearance => Outcome::Finding(
                Class::Clearance,
                format!(
                    "clearance clash with {counterpart}: separation {:.4} m below required {clearance:.4} m{note}",
                    measured.separation_metres()
                ),
            ),
            _ => Outcome::Pass,
        }
    }
}

/// A pair's outcome once an undecided exclusion is taken into account: it
/// never hides a finding and never reports one.
fn unless_excluded(
    outcome: Outcome,
    exclusion: Result<Option<String>, Unavailable>,
    counterpart: &ObjectId,
) -> Outcome {
    match (outcome, exclusion) {
        (Outcome::Pass, _) => Outcome::Pass,
        (_, Err(why)) => {
            let (reason, message) = excluded_words(&why, counterpart);
            Outcome::Open(reason, message)
        }
        (outcome, _) => outcome,
    }
}

/// Judges a measured pair with its profile, asking the tolerance cases
/// only when they can change the outcome, once at most; returns the
/// outcome and what the cases found.
fn judge_with_cases(
    profile: &Profile,
    declared: &Cases<'_>,
    cases: &mut CaseJudge<'_>,
    (service, measured): (&ProximityServiceHandle, &ProximityEvidence),
) -> (Outcome, Excuse) {
    let (subject, counterpart) = (
        measured.request().subject(),
        measured.request().counterpart(),
    );
    if declared.is_empty() {
        let outcome = profile.judge(measured, counterpart, &mut || (Holds::No, String::new()));
        return (outcome, Excuse::none());
    }
    let mut asked: Option<Excuse> = None;
    let outcome = profile.judge(measured, counterpart, &mut || {
        let excuse = asked.get_or_insert_with(|| cases.excuses(service, subject, counterpart));
        (excuse.holds, excuse.note.clone())
    });
    (outcome, asked.unwrap_or_else(Excuse::none))
}

impl Severities {
    /// A finding's severity, and a note for its message.
    ///
    /// An intersection's grade comes first, then `cell` (a clash matrix
    /// cell's own severity), then the class's, then `rule`'s.
    fn assign(
        &self,
        class: Class,
        measured: Option<&ProximityEvidence>,
        cell: Option<Severity>,
        rule: Severity,
    ) -> (Severity, String) {
        let fixed = cell
            .or_else(|| {
                self.by_class
                    .iter()
                    .find(|(known, _)| *known == class)
                    .map(|(_, severity)| severity.clone())
            })
            .unwrap_or(rule);
        let Some((measure, grades)) = &self.grades else {
            return (fixed, String::new());
        };
        if class != Class::Intersection {
            return (fixed, String::new());
        }
        let grade_of = |value: f64| {
            grades
                .iter()
                .rev()
                .find(|(above, _)| value > *above)
                .map_or_else(|| fixed.clone(), |(_, severity)| severity.clone())
        };
        let Some((lower, upper)) = measured.and_then(|measured| measure.interval(measured)) else {
            // Unmeasured: it may fall in any grade.
            let worst = grades
                .iter()
                .map(|(_, severity)| severity.clone())
                .chain([fixed])
                .min()
                .unwrap_or(Severity::Error);
            return (
                worst.clone(),
                format!(
                    ", graded {}: its {} is unmeasured, so the most severe grade it may reach",
                    label(&worst),
                    measure.name()
                ),
            );
        };
        // The grade of the lower bound, and of every bound the interval
        // reaches past: the grades a value in it may take.
        let worst = grades
            .iter()
            .filter(|(above, _)| *above >= lower && *above < upper)
            .map(|(_, severity)| severity.clone())
            .chain([grade_of(lower)])
            .min()
            .unwrap_or_else(|| grade_of(lower));
        let described = measure.describe((lower, upper));
        let note = if worst == grade_of(upper) && worst == grade_of(lower) {
            format!(
                ", graded {} by its {} of {described}",
                label(&worst),
                measure.name()
            )
        } else {
            format!(
                ", graded {}: the most severe grade its {} of {described} may reach",
                label(&worst),
                measure.name()
            )
        };
        (worst, note)
    }

    /// A reported pair's outcome, graded and, for a duplicate, with what
    /// the copies differ in: the outcome, its severity and the evidence of
    /// the quantities read.
    fn report(
        &self,
        context: &RuleContext<'_>,
        measured: &ProximityEvidence,
        pair: (&ObjectId, &ObjectId),
        outcome: Outcome,
        (cell, rule): (Option<Severity>, Severity),
    ) -> (Outcome, Severity, Vec<Evidence>) {
        let Outcome::Finding(class, mut message) = outcome else {
            return (outcome, rule, Vec::new());
        };
        let (severity, note) = self.assign(class, Some(measured), cell, rule);
        message.push_str(&note);
        let mut evidence = Vec::new();
        if class == Class::Duplicate {
            let (suffix, read) = self.differences(context, measured, pair);
            message.push_str(&suffix);
            evidence = read;
        }
        (Outcome::Finding(class, message), severity, evidence)
    }
}

/// One reported pair, before grouping.
struct Reported {
    subject: ObjectId,
    counterpart: ObjectId,
    class: Class,
    message: String,
    severity: Severity,
    evidence: Vec<Evidence>,
}

impl Reported {
    /// The pair's own finding: on its subject, relating its counterpart.
    fn finding(self, rule: &CompiledRule) -> Finding {
        Finding {
            explanation: None,
            id: None,
            decision: None,
            rule_id: rule.id.clone(),
            scope: Scope::Object(self.subject),
            severity: self.severity,
            message: self.message,
            related: Vec::new(),
            evidence: self.evidence,
            location: None,
            categories: Vec::new(),
        }
        .with_related([self.counterpart])
    }
}

/// Collects reported pairs into groups.
struct Groups<'r> {
    keys: GroupKeys<'r>,
    grouped: BTreeMap<Key, Vec<Reported>>,
    alone: Vec<Reported>,
}

impl<'r> Groups<'r> {
    fn new(context: &'r RuleContext<'r>, grouping: &'r Grouping<'r>) -> Result<Self, Unavailable> {
        Ok(Self {
            keys: GroupKeys::new(context, grouping)?,
            grouped: BTreeMap::new(),
            alone: Vec::new(),
        })
    }

    /// Adds a reported pair to its group, or on its own when its key
    /// cannot be read.
    fn add(&mut self, pair: &Context<'_>, mut reported: Reported) {
        let keyed = self
            .keys
            .key(
                pair,
                (&reported.subject, &reported.counterpart),
                reported.class == Class::Unmatched,
            )
            .map(|mut key| {
                if self.keys.grouping.by == GroupBy::Similar {
                    key.class = Some(reported.class);
                }
                key
            });
        match keyed {
            Ok(key) => self.grouped.entry(key).or_default().push(reported),
            Err(why) => {
                reported.message = format!("{} (not grouped: {why})", reported.message);
                self.alone.push(reported);
            }
        }
    }

    /// The group findings, then the pairs reported on their own.
    fn finish(self, rule: &CompiledRule) -> Vec<Finding> {
        let by = self.keys.grouping.by;
        let mut findings = Vec::new();
        for (key, mut members) in self.grouped {
            if members.len() == 1 {
                findings.extend(members.pop().map(|member| member.finding(rule)));
                continue;
            }
            findings.push(group_finding(rule, by, &key, members));
        }
        findings.extend(self.alone.into_iter().map(|member| member.finding(rule)));
        findings
    }
}

/// One finding for a group of pairs: on the object most of them involve,
/// relating every other, at the most severe of their severities.
fn group_finding(rule: &CompiledRule, by: GroupBy, key: &Key, members: Vec<Reported>) -> Finding {
    let mut counts: BTreeMap<&ObjectId, usize> = BTreeMap::new();
    for member in &members {
        *counts.entry(&member.subject).or_default() += 1;
        *counts.entry(&member.counterpart).or_default() += 1;
    }
    // The first of the most involved objects, in identity order.
    let most = counts.values().copied().max().unwrap_or(0);
    let hub = counts
        .iter()
        .find_map(|(object, count)| (*count == most).then(|| (*object).clone()))
        .unwrap_or_else(|| unreachable!("a group has members"));
    let severity = members
        .iter()
        .map(|member| member.severity.clone())
        .min()
        .unwrap_or_else(|| unreachable!("a group has members"));
    let related: Vec<ObjectId> = counts.keys().map(|object| (*object).clone()).collect();

    let count = members.len();
    let what = match (by, &key.sides) {
        (GroupBy::Similar, Some(_)) => format!(
            "{count} similar {} clashes of {}",
            key.class.map_or("", Class::name),
            key.sides_words()
        ),
        (GroupBy::TypePair, Some(_)) => format!("{count} clashes of {}", key.sides_words()),
        _ => format!("{count} clashes"),
    };
    let on = key.on_words();
    let mut evidence: Vec<Evidence> = Vec::new();
    let mut parts = Vec::with_capacity(count);
    for member in members {
        parts.push(format!("[{}] {}", member.subject, member.message));
        for entry in member.evidence {
            if !evidence.contains(&entry) {
                evidence.push(entry);
            }
        }
    }
    Finding {
        explanation: None,
        id: None,
        decision: None,
        rule_id: rule.id.clone(),
        scope: Scope::Object(hub),
        severity,
        message: format!("{what}{on}: {}", parts.join("; ")),
        related: Vec::new(),
        evidence,
        location: None,
        categories: Vec::new(),
    }
    .with_related(related)
}

/// Records a pair's outcome against its subject, or into its group.
struct Recorder<'r> {
    rule: &'r CompiledRule,
    evaluation: CapabilityEvaluation,
    unevaluated: Unevaluated,
    groups: Option<Groups<'r>>,
}

impl Recorder<'_> {
    fn record(
        &mut self,
        (subject, counterpart): (&ObjectId, &ObjectId),
        pair: &Context<'_>,
        outcome: Outcome,
        severity: Severity,
        evidence: Vec<Evidence>,
    ) {
        match outcome {
            Outcome::Finding(class, message) => {
                let reported = Reported {
                    subject: subject.clone(),
                    counterpart: counterpart.clone(),
                    class,
                    message,
                    severity,
                    evidence,
                };
                match &mut self.groups {
                    Some(groups) => groups.add(pair, reported),
                    None => self.evaluation.push_finding(reported.finding(self.rule)),
                }
            }
            Outcome::Open(reason, message) => {
                self.unevaluated.push(subject.clone(), reason, message);
            }
            Outcome::Pass => {}
        }
    }

    fn finish(self) -> CapabilityEvaluation {
        let Self {
            rule,
            mut evaluation,
            unevaluated,
            groups,
        } = self;
        for finding in groups.map(|groups| groups.finish(rule)).unwrap_or_default() {
            evaluation.push_finding(finding);
        }
        unevaluated.drain_into(&mut evaluation);
        evaluation
    }
}

/// The implementation `clash` replaced: each pair judged in code.
pub struct Clash;

impl RuleCapability for Clash {
    fn id(&self) -> &'static str {
        super::template::CLASH
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::clash_parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let prepared = match prepare(
            context,
            rule,
            Some(declared.profile.margin()),
            ProximityProjection::Minimum3d,
        ) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let mut exclusions = match Exclusions::new(
            context,
            &declared.exclude_paths,
            declared.exclude_target_property,
            declared.exclude_same_layer,
        ) {
            Ok(exclusions) => exclusions,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let groups = match declared
            .grouping
            .as_ref()
            .map(|grouping| Groups::new(context, grouping))
            .transpose()
        {
            Ok(groups) => groups,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let mut recorder = Recorder {
            rule,
            evaluation: CapabilityEvaluation::default(),
            unevaluated: prepared.unevaluated,
            groups,
        };
        let mut cases = CaseJudge::new(context, &declared.cases);
        for pair in &prepared.pairs {
            let (subject, counterpart) = (pair.subject(), pair.counterpart());
            let exclusion = exclusions.excluded(subject, counterpart);
            if matches!(exclusion, Ok(Some(_))) {
                continue;
            }
            let measured = match measure(prepared.service, subject, counterpart) {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    recorder.unevaluated.push(subject.clone(), reason, message);
                    continue;
                }
            };
            let (judged, excuse) = judge_with_cases(
                &declared.profile,
                &declared.cases,
                &mut cases,
                (prepared.service, &measured),
            );
            let (outcome, severity, read) = declared.severities.report(
                context,
                &measured,
                (subject, counterpart),
                judged,
                (None, severity(rule)),
            );
            let mut evidence = vec![measured.evidence().clone()];
            evidence.extend(read);
            if matches!(outcome, Outcome::Finding(..)) {
                evidence.extend(excuse.evidence);
            }
            recorder.record(
                (subject, counterpart),
                &Context {
                    measured: Some(&measured),
                    cell: None,
                },
                unless_excluded(outcome, exclusion, counterpart),
                severity,
                evidence,
            );
        }
        recorder.finish()
    }
}

/// The implementation `clash-matrix` replaced: each pair's cell chosen and
/// the pair judged in code.
pub struct ClashMatrix;

impl RuleCapability for ClashMatrix {
    fn id(&self) -> &'static str {
        super::template::CLASH_MATRIX
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        crate::clash_matrix::parameters()
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match crate::clash_matrix::declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let margin = declared
            .cells
            .iter()
            .map(|cell| cell.profile.margin())
            .fold(0.0, f64::max);
        let prepared = match prepare(context, rule, Some(margin), ProximityProjection::Minimum3d) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let mut exclusions = match Exclusions::new(
            context,
            &declared.exclude_paths,
            declared.exclude_target_property,
            declared.exclude_same_layer,
        ) {
            Ok(exclusions) => exclusions,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let groups = match declared
            .grouping
            .as_ref()
            .map(|grouping| Groups::new(context, grouping))
            .transpose()
        {
            Ok(groups) => groups,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let mut categories = Categories::new(context, &declared);
        let mut recorder = Recorder {
            rule,
            evaluation: CapabilityEvaluation::default(),
            unevaluated: prepared.unevaluated,
            groups,
        };
        let indices: Vec<usize> = (0..declared.cells.len()).collect();
        let mut cases = CaseJudge::new(context, &declared.cases);

        for pair in &prepared.pairs {
            let (subject, counterpart) = (pair.subject(), pair.counterpart());
            let (Some(subject_object), Some(counterpart_object)) = (
                context.project.object(subject),
                context.project.object(counterpart),
            ) else {
                recorder.unevaluated.push(
                    subject.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    format!("the pair with {counterpart} names an object outside the project"),
                );
                continue;
            };
            let exclusion = exclusions.excluded(subject, counterpart);
            if matches!(exclusion, Ok(Some(_))) {
                continue;
            }
            let mut unknown = Vec::new();
            let matched = match_rows(&indices, RowSelection::MostSpecific, |&cell| {
                categories.test_pair(cell, subject_object, counterpart_object, &mut unknown)
            });
            let (index, cell) = match matched {
                Matched::Rows(rows) if rows.is_empty() => {
                    if declared.report_unmatched {
                        let mut evidence = categories.category(subject_object).evidence.clone();
                        evidence.extend(
                            categories
                                .category(counterpart_object)
                                .evidence
                                .iter()
                                .cloned(),
                        );
                        let outcome = Outcome::Finding(
                            Class::Unmatched,
                            format!(
                                "no clash matrix cell covers {} against {}",
                                categories.describe(subject_object),
                                categories.describe(counterpart_object),
                            ),
                        );
                        recorder.record(
                            (subject, counterpart),
                            &Context {
                                measured: None,
                                cell: None,
                            },
                            unless_excluded(outcome, exclusion, counterpart),
                            severity(rule),
                            evidence,
                        );
                    }
                    continue;
                }
                Matched::Rows(rows) => {
                    let (_, &index) = rows[0];
                    (index, &declared.cells[index])
                }
                Matched::Undecided => {
                    let (reason, message) = unknown.into_iter().next().unwrap_or((
                        NotEvaluatedReason::IncompleteEvidence,
                        "a category cannot be read".into(),
                    ));
                    // A fact the source records for nothing is about the
                    // source: keep the message free of object names, so the
                    // runtime reports it once per source.
                    let message = if reason == NotEvaluatedReason::NotRecorded {
                        format!("the clash matrix cell cannot be chosen: {message}")
                    } else {
                        format!(
                            "the clash matrix cell for the pair with {counterpart} cannot be chosen: {message}"
                        )
                    };
                    recorder.unevaluated.push(subject.clone(), reason, message);
                    continue;
                }
                Matched::Ambiguous(tied) => {
                    let names: Vec<String> = tied
                        .iter()
                        .map(|&index| declared.cells[index].name(index))
                        .collect();
                    recorder.unevaluated.push(
                        subject.clone(),
                        NotEvaluatedReason::InvalidDeclaration,
                        format!(
                            "clash matrix {} cover the pair with {counterpart} equally",
                            names.join(" and ")
                        ),
                    );
                    continue;
                }
            };
            if !cell.profile.checks_anything() {
                continue;
            }
            let measured = match measure(prepared.service, subject, counterpart) {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    recorder.unevaluated.push(subject.clone(), reason, message);
                    continue;
                }
            };
            let (judged, excuse) = judge_with_cases(
                &cell.profile,
                &declared.cases,
                &mut cases,
                (prepared.service, &measured),
            );
            let (outcome, severity, mut read) = declared.severities.report(
                context,
                &measured,
                (subject, counterpart),
                judged,
                (cell.severity.clone(), severity(rule)),
            );
            if matches!(outcome, Outcome::Finding(..)) {
                read.extend(excuse.evidence);
            }
            let outcome = match outcome {
                Outcome::Finding(class, message) => Outcome::Finding(
                    class,
                    format!("{message} (clash matrix {})", cell.name(index)),
                ),
                other => other,
            };
            let mut evidence = vec![measured.evidence().clone()];
            evidence.extend(categories.category(subject_object).evidence.iter().cloned());
            evidence.extend(
                categories
                    .category(counterpart_object)
                    .evidence
                    .iter()
                    .cloned(),
            );
            evidence.extend(read);
            evidence.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
            evidence.dedup();
            recorder.record(
                (subject, counterpart),
                &Context {
                    measured: Some(&measured),
                    cell: Some(index),
                },
                unless_excluded(outcome, exclusion, counterpart),
                severity,
                evidence,
            );
        }
        recorder.finish()
    }
}
