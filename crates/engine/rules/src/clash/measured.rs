//! The candidate pairs `clash` and `clash-matrix` judge, as measured member
//! lists of the project (`clash_pairs`, `clash_matrix_pairs`).
//!
//! The broad phase proposes the pairs of the subjects and their
//! counterparts within the widest clearance; each pair surely excluded is
//! left out, and every other is measured once by the proximity service
//! (separation, penetration, containment, Hausdorff distance, overlap
//! extents, shared volume) and listed beside the tolerances it is judged
//! against: the rule's, or the matrix cell covering it most specifically.
//! An object whose extent cannot be read, a pair that cannot be measured
//! and one whose cell cannot be chosen are listed open, with why and for
//! which reason. Nothing here decides a class: the template puts each pair
//! in one, against the tolerances listed with it.
//!
//! The list also states what words a finding on the pair needs and what
//! keys group it: what two duplicates differ in, whether a tolerance case
//! along the elements' own axes excuses an intersection (measured only
//! for a pair penetrating past its tolerance, which an intersection
//! needs), and the pair's group key.

use std::collections::BTreeMap;

use axioval_engine::{
    BodyContainment, LengthInterval, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    MetricDirection, ObjectFrameServiceHandle, OverlapAlongRequest, PropertyResolutionError,
    ProximityError, ProximityEvidence, ProximityProjection, ProximityRequest,
    ProximityServiceHandle, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, QuantityDimension};

use super::{Exclusions, Holds, Profile, declaration, described, described_volume, excluded_words};
use crate::clash_cases::{Cases, Excuse, described as described_extent, lesser};
use crate::clash_groups::{Context, GroupKeys};
use crate::clash_matrix::Categories;
use crate::clash_severity::{Severities, label};
use crate::pairs::{Unevaluated, fidelity_note, prepare_among, reason};
use crate::selection::{Selection, selector_matches};
use crate::support::Unavailable;
use crate::support::table::{Matched, RowSelection, match_rows};

/// Measures the member lists `clash_pairs` and `clash_matrix_pairs`.
pub struct ClashPairs;

impl MeasuredProvider for ClashPairs {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["clash_matrix_pairs", "clash_pairs"]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        _object: &ObjectId,
        _context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::MissingService(format!(
            "`{}` is a member list of the project",
            call.name()
        )))
    }

    fn members_of_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let members = if call.name() == "clash_matrix_pairs" {
            matrix_pairs(call, context)?
        } else {
            clash_pairs(call, context)?
        };
        Ok((members, Vec::new()))
    }
}

/// Measures one pair, refusing a measurement that names another pair.
pub(crate) fn measure(
    service: &ProximityServiceHandle,
    subject: &ObjectId,
    counterpart: &ObjectId,
) -> Result<ProximityEvidence, Unavailable> {
    ProximityRequest::try_new(subject.clone(), counterpart.clone())
        .and_then(|request| service.measure_proximity(&request))
        .and_then(|measured| {
            // A measurement of another pair answers a different question.
            if measured.request().subject() == subject
                && measured.request().counterpart() == counterpart
            {
                Ok(measured)
            } else {
                Err(ProximityError::InvalidMeasurement)
            }
        })
        .map_err(|error| {
            (
                reason(error),
                format!("proximity to {counterpart} could not be measured: {error}"),
            )
        })
}

/// The rule parameters a list names, by their own names, as the rule
/// states them: what the capabilities' declarations read.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Length(value) | MeasuredArgument::Number(value) => {
                    ParameterValue::Number { value: *value }
                }
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
                MeasuredArgument::Text(value) => ParameterValue::String {
                    value: value.clone(),
                },
                MeasuredArgument::Choice(value) => ParameterValue::String {
                    value: (*value).to_owned(),
                },
                MeasuredArgument::Path(paths) => ParameterValue::StringList {
                    value: paths.clone(),
                },
                MeasuredArgument::Property { set, name } => ParameterValue::PropertyReference {
                    property: name.clone(),
                    property_set: set.clone(),
                },
                MeasuredArgument::Table(rows) => ParameterValue::Table {
                    value: rows.clone(),
                },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect()
}

/// The objects a list's `key` names: those its selection surely picks, or
/// those of the source kinds it writes, in identity order.
fn objects(
    call: &MeasuredCall,
    key: &str,
    context: &RuleContext<'_>,
) -> Result<Vec<ObjectId>, PropertyResolutionError> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => {
            Ok(selection.matched.iter().cloned().collect())
        }
        Some(MeasuredArgument::SourceKind(_)) => {
            crate::measured_kinds::every_object_of_kinds(context, call, key)
                .map(|found| found.into_iter().collect())
        }
        _ => Err(PropertyResolutionError::InvalidArgument(format!(
            "`{}` names no `{key}`",
            call.name()
        ))),
    }
}

fn refused((_, message): Unavailable) -> PropertyResolutionError {
    PropertyResolutionError::InvalidArgument(message)
}

/// What both lists read before their pairs: the proximity service and the
/// broad phase's pairs, the objects it could not measure listed open.
struct Proposed<'s> {
    service: &'s ProximityServiceHandle,
    pairs: Vec<axioval_engine::CandidatePair>,
    members: Listed,
}

/// The items listed so far, each object's open outcome once: a run holds
/// them all, and the template reports each once however often it is
/// listed.
#[derive(Default)]
struct Listed {
    items: Vec<MeasuredMember>,
    opened: std::collections::BTreeSet<(ObjectId, NotEvaluatedReason, String)>,
}

impl Listed {
    /// An item open on `object`, unless one is listed with these words.
    fn open(&mut self, object: &ObjectId, reason: &NotEvaluatedReason, message: String) {
        let key = (object.clone(), reason.clone(), message);
        if self.opened.contains(&key) {
            return;
        }
        self.items.push(open(object, reason, key.2.clone()));
        self.opened.insert(key);
    }

    fn push(&mut self, member: MeasuredMember) {
        self.items.push(member);
    }
}

fn propose<'s>(
    call: &MeasuredCall,
    context: &RuleContext<'s>,
    margin: f64,
) -> Result<Proposed<'s>, PropertyResolutionError> {
    let subjects = objects(call, "subjects", context)?;
    let counterparts = objects(call, "counterparts", context)?;
    let found = |ids: &[ObjectId]| -> Vec<&'s axioval_ir::Object> {
        ids.iter()
            .filter_map(|id| crate::selection::object_by_id(context, id))
            .collect()
    };
    let prepared = prepare_among(
        context,
        (&found(&subjects), &found(&counterparts)),
        Unevaluated::default(),
        (margin, ProximityProjection::Minimum3d),
    )
    .map_err(|(reason, message)| match reason {
        NotEvaluatedReason::MissingService => PropertyResolutionError::MissingService(message),
        _ => PropertyResolutionError::Conflicting(message),
    })?;
    let mut members = Listed::default();
    for (object, reason, message) in prepared.unevaluated.iter() {
        members.open(object, reason, message.clone());
    }
    Ok(Proposed {
        service: prepared.service,
        pairs: prepared.pairs,
        members,
    })
}

/// A not-evaluated reason as reports spell it.
fn spelled(reason: &NotEvaluatedReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

fn text(text: impl Into<String>) -> MemberValue {
    MemberValue::Text { text: text.into() }
}

fn objects_of(objects: &[&ObjectId]) -> MemberValue {
    MemberValue::Objects {
        objects: objects.iter().map(|object| (*object).clone()).collect(),
    }
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: String::new(),
    }
}

fn number((lower, upper): (f64, f64), dimension: QuantityDimension, locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Value {
        lower,
        upper,
        dimension: Some(dimension),
        locator: locator.to_owned(),
    })
}

fn unmeasured() -> MemberValue {
    MemberValue::Undecided {
        why: "unmeasured".into(),
    }
}

fn length(interval: Option<LengthInterval>, locator: &str) -> MemberValue {
    interval.map_or_else(unmeasured, |interval| {
        number(
            (interval.lower_metres(), interval.upper_metres()),
            QuantityDimension::Length,
            locator,
        )
    })
}

/// An item open on `object` (and the pair with `counterpart`), with why
/// and for which reason.
fn open(object: &ObjectId, reason: &NotEvaluatedReason, message: String) -> MeasuredMember {
    MeasuredMember {
        certain: true,
        exact: true,
        fields: BTreeMap::from([
            ("subject", objects_of(&[object])),
            ("open", text(message)),
            ("reason", text(spelled(reason))),
        ]),
        evidence: Vec::new(),
    }
}

/// The fields stating whether a pair may be excluded.
fn exclusion(
    fields: &mut BTreeMap<&'static str, MemberValue>,
    excluded: &Result<Option<String>, Unavailable>,
    counterpart: &ObjectId,
) {
    if let Err(why) = excluded {
        let (reason, words) = excluded_words(why, counterpart);
        fields.insert("excluded", text(words));
        fields.insert("excluded_reason", text(spelled(&reason)));
    }
}

/// A text field, where it states words.
fn put(fields: &mut BTreeMap<&'static str, MemberValue>, name: &'static str, words: String) {
    if !words.is_empty() {
        fields.insert(name, text(words));
    }
}

/// What a pair's list reads beside its measurement.
struct Judging<'a, 'r> {
    context: &'r RuleContext<'r>,
    service: &'r ProximityServiceHandle,
    severities: &'a Severities,
    cases: &'a Cases<'a>,
    judge: CaseJudge<'r>,
    keys: Option<GroupKeys<'r>>,
}

impl Judging<'_, '_> {
    /// The fields of a measured pair judged against `profile`.
    #[allow(clippy::too_many_lines)]
    fn pair(
        &mut self,
        measured: &ProximityEvidence,
        profile: &Profile,
        cell: Option<usize>,
    ) -> MeasuredMember {
        let (subject, counterpart) = (
            measured.request().subject(),
            measured.request().counterpart(),
        );
        // The member cites the measurement itself (`evidence`); its fields
        // carry no locator of their own.
        let locator = "";
        // What the pair was measured from: the proximity measurement, and
        // what a duplicate's comparison or a tolerance case read.
        let mut evidence = vec![measured.evidence().clone()];
        let metres = |value: f64| number((value, value), QuantityDimension::Length, "");
        // A pair is listed with what it states: a text without words, a
        // truth that is false, a switch that is on and a value that is
        // `null` are left out, as the list reads them (a run holds many
        // pairs at once).
        let mut fields = BTreeMap::from([
            ("subject", objects_of(&[subject])),
            ("counterpart", objects_of(&[counterpart])),
            (
                "separation",
                number(
                    (measured.separation_metres(), measured.separation_metres()),
                    QuantityDimension::Length,
                    locator,
                ),
            ),
        ]);
        // A cell's tolerances; a rule's are its parameters, the same for
        // every pair.
        if cell.is_some() {
            fields.extend([
                (
                    "penetration_tolerance",
                    metres(profile.penetration_tolerance),
                ),
                ("duplicate_tolerance", metres(profile.duplicate_tolerance)),
                ("horizontal_tolerance", metres(profile.horizontal_tolerance)),
                ("vertical_tolerance", metres(profile.vertical_tolerance)),
                (
                    "volume_tolerance",
                    number(
                        (profile.volume_tolerance, profile.volume_tolerance),
                        QuantityDimension::Volume,
                        "",
                    ),
                ),
            ]);
            if let Some(clearance) = profile.clearance {
                fields.insert("clearance", metres(clearance));
            }
        }
        if let Some(certified) = measured.certified_separation() {
            fields.insert("certified", length(Some(certified), locator));
        }
        if let Some(depth) = measured.penetration_metres() {
            fields.insert(
                "penetration",
                number((depth, depth), QuantityDimension::Length, locator),
            );
        }
        match measured.containment() {
            Some(BodyContainment::SubjectInsideCounterpart) => {
                fields.insert("inside", truth(true));
            }
            Some(BodyContainment::CounterpartInsideSubject) => {
                fields.insert("contains", truth(true));
            }
            None => {}
        }
        put(&mut fields, "note", fidelity_note(measured.fidelity()));
        for (name, on) in [
            ("report_duplicates", profile.report.duplicate),
            ("report_containment", profile.report.containment),
            ("report_intersections", profile.report.intersection),
        ] {
            if !on {
                fields.insert(name, truth(false));
            }
        }
        // The surfaces' distance: at least the separation, no upper end
        // where it was not measured.
        let hausdorff = measured.hausdorff_interval_metres();
        let lower = hausdorff
            .map_or(0.0, |interval| interval.lower_metres())
            .max(measured.separation_interval_metres().0);
        let upper = hausdorff.map_or(f64::INFINITY, |interval| interval.upper_metres());
        fields.insert(
            "hausdorff",
            number((lower, upper), QuantityDimension::Length, locator),
        );
        if !upper.is_finite() {
            fields.insert("hausdorff_upper", text("an unmeasured distance"));
        }
        let extents = measured.overlap_extents();
        fields.insert(
            "horizontal",
            length(extents.map(|extents| extents.horizontal()), locator),
        );
        fields.insert(
            "vertical",
            length(extents.map(|extents| extents.vertical()), locator),
        );
        fields.insert(
            "smallest_extent",
            length(
                extents.map(|extents| lesser(lesser(extents.x(), extents.y()), extents.z())),
                locator,
            ),
        );
        let shared = measured.intersection_volume().map(|volume| volume.shared());
        fields.insert(
            "shared_volume",
            shared.map_or_else(unmeasured, |shared| {
                number(
                    (shared.lower_cubic_metres(), shared.upper_cubic_metres()),
                    QuantityDimension::Volume,
                    locator,
                )
            }),
        );
        // The intersection's reach, as the tolerances declare it.
        let mut reach = String::new();
        if profile.horizontal_tolerance > 0.0 || profile.vertical_tolerance > 0.0 {
            reach = format!(
                ", reaching {} in plan and {} vertically",
                described(extents.map(|extents| extents.horizontal())),
                described(extents.map(|extents| extents.vertical()))
            );
        }
        if profile.volume_tolerance > 0.0 {
            reach = format!("{reach}, sharing {}", described_volume(shared));
        }
        put(&mut fields, "reach", reach);
        // What two duplicates differ in, read only where the pair may be
        // a reported duplicate.
        let copies = if profile.report.duplicate && lower <= profile.duplicate_tolerance {
            let (copies, read) =
                self.severities
                    .differences(self.context, measured, (subject, counterpart));
            evidence.extend(read);
            copies
        } else {
            String::new()
        };
        put(&mut fields, "copies", copies);
        // A tolerance case is measured only for a pair penetrating past its
        // tolerance, the only one an intersection may be.
        let penetrating = measured.containment().is_none()
            && measured
                .penetration_metres()
                .is_some_and(|depth| depth > profile.penetration_tolerance);
        let excuse = if penetrating && profile.report.intersection && !self.cases.is_empty() {
            self.judge.excuses(self.service, subject, counterpart)
        } else {
            Excuse::none()
        };
        match excuse.holds {
            Holds::Yes => {
                fields.insert("excused", truth(true));
            }
            Holds::Unknown => {
                fields.insert(
                    "excused",
                    MemberValue::Undecided {
                        why: excuse.note.clone(),
                    },
                );
            }
            Holds::No => {}
        }
        put(&mut fields, "case", excuse.note);
        evidence.extend(excuse.evidence);
        self.group(
            &mut fields,
            &Context {
                measured: Some(measured),
                cell,
            },
            (subject, counterpart),
            false,
        );
        MeasuredMember {
            certain: true,
            exact: evidence.iter().all(|evidence| evidence.exact),
            fields,
            evidence,
        }
    }

    /// The fields keying the pair's group, where findings are grouped; its
    /// types and storeys left out where they state no words, as every text
    /// is.
    fn group(
        &mut self,
        fields: &mut BTreeMap<&'static str, MemberValue>,
        pair: &Context<'_>,
        (subject, counterpart): (&ObjectId, &ObjectId),
        unmatched: bool,
    ) {
        let Some(keys) = &mut self.keys else {
            return;
        };
        match keys.key(pair, (subject, counterpart), unmatched) {
            Ok(key) => {
                fields.insert("group", text(key.encoded()));
                put(fields, "sides", key.sides_words());
                put(fields, "on", key.on_words());
            }
            Err(why) => {
                fields.insert("group", MemberValue::Undecided { why });
            }
        }
    }
}

/// `clash_pairs`: every candidate pair measured beside the rule's
/// tolerances.
fn clash_pairs(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let rule = super::synthesised(stated(call));
    let declared = declaration(&rule).map_err(refused)?;
    let Proposed {
        service,
        pairs,
        mut members,
    } = propose(call, context, declared.profile.margin())?;
    let mut exclusions = Exclusions::new(
        context,
        &declared.exclude_paths,
        declared.exclude_target_property,
        declared.exclude_same_layer,
    )
    .map_err(refused)?;
    let keys = declared
        .grouping
        .as_ref()
        .map(|grouping| GroupKeys::new(context, grouping))
        .transpose()
        .map_err(refused)?;
    let mut judging = Judging {
        context,
        service,
        severities: &declared.severities,
        cases: &declared.cases,
        judge: CaseJudge::new(context, &declared.cases),
        keys,
    };
    for pair in &pairs {
        let (subject, counterpart) = (pair.subject(), pair.counterpart());
        let excluded = exclusions.excluded(subject, counterpart);
        if matches!(excluded, Ok(Some(_))) {
            continue;
        }
        let measured = match measure(service, subject, counterpart) {
            Ok(measured) => measured,
            Err((reason, message)) => {
                members.open(subject, &reason, message);
                continue;
            }
        };
        let mut member = judging.pair(&measured, &declared.profile, None);
        exclusion(&mut member.fields, &excluded, counterpart);
        members.push(member);
    }
    Ok(members.items)
}

/// `clash_matrix_pairs`: every candidate pair a cell covers, measured
/// beside its cell's tolerances; a pair no cell covers where the rule
/// reports it.
#[allow(clippy::too_many_lines)]
fn matrix_pairs(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let rule = super::synthesised(stated(call));
    let declared = crate::clash_matrix::declaration(&rule).map_err(refused)?;
    let margin = declared
        .cells
        .iter()
        .map(|cell| cell.profile.margin())
        .fold(0.0, f64::max);
    let Proposed {
        service,
        pairs,
        mut members,
    } = propose(call, context, margin)?;
    let mut exclusions = Exclusions::new(
        context,
        &declared.exclude_paths,
        declared.exclude_target_property,
        declared.exclude_same_layer,
    )
    .map_err(refused)?;
    let keys = declared
        .grouping
        .as_ref()
        .map(|grouping| GroupKeys::new(context, grouping))
        .transpose()
        .map_err(refused)?;
    let mut categories = Categories::new(context, &declared);
    let mut judging = Judging {
        context,
        service,
        severities: &declared.severities,
        cases: &declared.cases,
        judge: CaseJudge::new(context, &declared.cases),
        keys,
    };
    let indices: Vec<usize> = (0..declared.cells.len()).collect();
    for pair in &pairs {
        let (subject, counterpart) = (pair.subject(), pair.counterpart());
        let (Some(subject_object), Some(counterpart_object)) = (
            context.project.object(subject),
            context.project.object(counterpart),
        ) else {
            members.open(
                subject,
                &NotEvaluatedReason::InvalidEvidence,
                format!("the pair with {counterpart} names an object outside the project"),
            );
            continue;
        };
        let excluded = exclusions.excluded(subject, counterpart);
        if matches!(excluded, Ok(Some(_))) {
            continue;
        }
        let mut unknown = Vec::new();
        let matched = match_rows(&indices, RowSelection::MostSpecific, |&cell| {
            categories.test_pair(cell, subject_object, counterpart_object, &mut unknown)
        });
        let (index, covering) = match matched {
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
                    let mut fields = BTreeMap::from([
                        ("subject", objects_of(&[subject])),
                        ("counterpart", objects_of(&[counterpart])),
                        ("unmatched", truth(true)),
                        (
                            "subject_category",
                            text(categories.describe(subject_object)),
                        ),
                        (
                            "counterpart_category",
                            text(categories.describe(counterpart_object)),
                        ),
                    ]);
                    exclusion(&mut fields, &excluded, counterpart);
                    judging.group(
                        &mut fields,
                        &Context {
                            measured: None,
                            cell: None,
                        },
                        (subject, counterpart),
                        true,
                    );
                    members.push(MeasuredMember {
                        certain: true,
                        exact: evidence.iter().all(|evidence| evidence.exact),
                        fields,
                        evidence,
                    });
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
                members.open(subject, &reason, message);
                continue;
            }
            Matched::Ambiguous(tied) => {
                let names: Vec<String> = tied
                    .iter()
                    .map(|&index| declared.cells[index].name(index))
                    .collect();
                members.open(
                    subject,
                    &NotEvaluatedReason::InvalidDeclaration,
                    format!(
                        "clash matrix {} cover the pair with {counterpart} equally",
                        names.join(" and ")
                    ),
                );
                continue;
            }
        };
        if !covering.profile.checks_anything() {
            continue;
        }
        let measured = match measure(service, subject, counterpart) {
            Ok(measured) => measured,
            Err((reason, message)) => {
                members.open(subject, &reason, message);
                continue;
            }
        };
        let mut member = judging.pair(&measured, &covering.profile, Some(index));
        member
            .evidence
            .extend(categories.category(subject_object).evidence.iter().cloned());
        member.evidence.extend(
            categories
                .category(counterpart_object)
                .evidence
                .iter()
                .cloned(),
        );
        member.exact = member.evidence.iter().all(|evidence| evidence.exact);
        exclusion(&mut member.fields, &excluded, counterpart);
        member.fields.insert(
            "cell",
            text(format!(" (clash matrix {})", covering.name(index))),
        );
        if let Some(severity) = &covering.severity {
            member.fields.insert("severity", text(label(severity)));
        }
        members.push(member);
    }
    Ok(members.items)
}

type Axes = [MetricDirection; 3];

/// Judges the declared tolerance cases against pairs, reading each frame
/// and selection once.
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
                            NotEvaluatedReason::MissingService,
                            "object-frame service is not registered".to_owned(),
                        )
                    })?;
                let placed = service.object_frame(object).map_err(|error| {
                    (
                        NotEvaluatedReason::IncompleteEvidence,
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
                            described_extent(extent)
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
