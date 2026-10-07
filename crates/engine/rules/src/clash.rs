//! Source-neutral clash and clearance capability.
//!
//! ADR 0004: proximity is measured by a [`axioval_engine::ProximityServiceHandle`];
//! whether a measured overlap is a clash is decided by the template, against
//! declared tolerances.
//!
//! Each pair falls into one class, tried in this order:
//!
//! - **Duplicate**: the two surfaces lie within `duplicate_tolerance_metres`
//!   of each other (their Hausdorff distance), whatever else is true of them.
//! - **Containment**: one body lies wholly inside the other.
//! - **Intersection**: one body reaches into the other deeper than the
//!   penetration tolerance, the intersection reaches further than the
//!   horizontal tolerance along both plan axes and further than the vertical
//!   tolerance in height, and the volume the bodies share exceeds the volume
//!   tolerance. Zero separation alone is not a clash: a slab resting on a
//!   wall has its surfaces meeting and nothing interpenetrating.
//! - **Clearance**: two bodies coming closer than the declared clearance
//!   without falling into a class above.
//!
//! Each of the first three classes has a switch. A switched-off class is not
//! reported, and its pairs are not reported as anything else either: a
//! duplicate is not an intersection.
//!
//! Every comparison is made on the measured interval. A class holds when the
//! whole interval says so and fails when none of it does; a pair whose
//! interval straddles a tolerance is reported only when both readings lead to
//! a finding, and is otherwise not evaluated.
//!
//! Pairs can be excluded: pairs whose objects reach a shared target through a
//! declared relationship path (the same system, the same parent element,
//! connected ports), and pairs on a shared presentation layer. An exclusion
//! that cannot be decided leaves a pair that would be reported not evaluated.
//!
//! When neither body is a closed solid there is no inside to measure, so
//! surfaces that meet cannot be classified: the pair is reported not
//! evaluated rather than passed. Measurements on tessellated geometry are
//! reported, and marked approximate in both the message and the evidence.
//!
//! It runs as a template ([`axioval_engine::template`]): the broad phase,
//! the exclusions and each pair's measurement are the measured list
//! `clash_pairs` ([`measured`]), and the template puts each pair in its
//! class, grades and groups it ([`axioval_engine::template::Pairs`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Object, ObjectId, PRESENTATION_LAYER, PRESENTATION_SET, PropertyValue};

use crate::clash_cases::{Cases, cases};
use crate::clash_groups::{Grouping, grouping};
use crate::clash_severity::{Severities, severities};
use crate::selection::property_error;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, invalid, resolve, undefined,
    value_key,
};

pub(crate) mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
pub(crate) mod template;

pub(crate) use measured::{ClashPairs, measure};

/// Reports duplicates, contained bodies, intersections and clearance
/// shortfalls between bodies.
///
/// It runs as a template: the measured list `clash_pairs` lists every
/// candidate pair measured once beside the rule's tolerances, and the
/// template judges each by the first class that holds.
pub struct Clash;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::clash);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for Clash {
    fn id(&self) -> &'static str {
        template::CLASH
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// Which classes are reported.
pub(crate) struct Report {
    pub(crate) duplicate: bool,
    pub(crate) containment: bool,
    pub(crate) intersection: bool,
}

/// The tolerances a pair is judged against and the classes it reports: a
/// rule's parameters for `clash`, one cell's for `clash-matrix`.
pub(crate) struct Profile {
    pub(crate) penetration_tolerance: f64,
    pub(crate) clearance: Option<f64>,
    pub(crate) duplicate_tolerance: f64,
    pub(crate) horizontal_tolerance: f64,
    pub(crate) vertical_tolerance: f64,
    pub(crate) volume_tolerance: f64,
    pub(crate) report: Report,
}

/// Reads one named value; absent is `None`, another type an error.
pub(crate) type Read<'f, T> = &'f dyn Fn(&str) -> Result<Option<T>, Unavailable>;

impl Profile {
    /// Reads a profile by name from a rule's parameters or a table row's
    /// cells, which name its values alike.
    pub(crate) fn read(
        number: Read<'_, f64>,
        boolean: Read<'_, bool>,
    ) -> Result<Self, Unavailable> {
        let length = |name: &str| -> Result<Option<f64>, Unavailable> {
            match number(name)? {
                Some(value) if value < 0.0 => {
                    Err(invalid(format!("`{name}` must not be negative")))
                }
                other => Ok(other),
            }
        };
        let penetration_tolerance = length("penetration_tolerance_metres")?
            .ok_or_else(|| invalid("`penetration_tolerance_metres` is required"))?;
        let clearance = length("clearance_metres")?;
        if clearance == Some(0.0) {
            return Err(invalid("`clearance_metres` must be positive"));
        }
        let switch =
            |name: &str| -> Result<bool, Unavailable> { Ok(boolean(name)?.unwrap_or(true)) };
        Ok(Self {
            penetration_tolerance,
            clearance,
            duplicate_tolerance: length("duplicate_tolerance_metres")?.unwrap_or(0.0),
            horizontal_tolerance: length("horizontal_tolerance_metres")?.unwrap_or(0.0),
            vertical_tolerance: length("vertical_tolerance_metres")?.unwrap_or(0.0),
            volume_tolerance: length("volume_tolerance_cubic_metres")?.unwrap_or(0.0),
            report: Report {
                duplicate: switch("report_duplicates")?,
                containment: switch("report_containment")?,
                intersection: switch("report_intersections")?,
            },
        })
    }

    /// Whether a class is reported or a clearance declared.
    pub(crate) fn checks_anything(&self) -> bool {
        self.report.duplicate
            || self.report.containment
            || self.report.intersection
            || self.clearance.is_some()
    }

    /// How far apart the broad phase must still propose a pair.
    pub(crate) fn margin(&self) -> f64 {
        self.clearance.unwrap_or(0.0)
    }
}

/// The profile's value names, shared by the `clash` parameters and the
/// `clash-matrix` cell columns.
pub(crate) const PROFILE_NUMBERS: [&str; 6] = [
    "penetration_tolerance_metres",
    "clearance_metres",
    "duplicate_tolerance_metres",
    "horizontal_tolerance_metres",
    "vertical_tolerance_metres",
    "volume_tolerance_cubic_metres",
];
pub(crate) const PROFILE_SWITCHES: [&str; 3] = [
    "report_duplicates",
    "report_containment",
    "report_intersections",
];

/// A `clash` rule's declaration, read as the capability reads it: by the
/// pairs' measured list and, refusing it, by the template's declaration
/// check.
pub(crate) struct Declaration<'a> {
    pub(crate) profile: Profile,
    /// Relationship paths, each a list of steps.
    pub(crate) exclude_paths: Vec<Vec<String>>,
    pub(crate) exclude_target_property: Option<PropertyRef<'a>>,
    pub(crate) exclude_same_layer: bool,
    pub(crate) grouping: Option<Grouping<'a>>,
    #[cfg_attr(not(feature = "parity-reference"), allow(dead_code))]
    pub(crate) severities: Severities,
    pub(crate) cases: Cases<'a>,
}

/// The `exclude_paths` parameter, each entry split into its steps.
pub(crate) fn exclusion_paths(
    parameters: &Parameters<'_>,
) -> Result<Vec<Vec<String>>, Unavailable> {
    let paths: Vec<Vec<String>> = parameters
        .strings("exclude_paths")?
        .unwrap_or_default()
        .iter()
        .map(|path| path.split_whitespace().map(str::to_owned).collect())
        .collect();
    for path in &paths {
        if path.is_empty() {
            return Err(invalid("an `exclude_paths` entry has no steps"));
        }
        Traversal::path(path)?;
    }
    Ok(paths)
}

/// Reads a `clash` rule's declaration, refusing it in the capability's
/// order and words.
pub(crate) fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let profile = Profile::read(&|name| parameters.number(name), &|name| {
        parameters.boolean(name)
    })?;
    if !profile.checks_anything() {
        return Err(invalid(
            "every class is switched off and no clearance is declared: nothing is checked",
        ));
    }
    Ok(Declaration {
        profile,
        exclude_paths: exclusion_paths(&parameters)?,
        exclude_target_property: exclusion_property(&parameters)?,
        exclude_same_layer: parameters.boolean("exclude_same_layer")?.unwrap_or(false),
        grouping: grouping(&parameters)?,
        severities: severities(&parameters)?,
        cases: cases(&parameters)?,
    })
}

/// The `exclude_target_property` parameter: targets reached through an
/// exclusion path also meet when they state the same value of it.
pub(crate) fn exclusion_property<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<PropertyRef<'a>>, Unavailable> {
    parameters.property("exclude_target_property")
}

/// The rule the parameters `stated` (keyed by their own names) declare,
/// as a declaration reads them.
pub(crate) fn synthesised(stated: BTreeMap<String, ParameterValue>) -> CompiledRule {
    crate::light_area::synthesised(stated)
}

/// The declaration the capability refused, in its order and words: what
/// the template's `Check::Arguments` refuses once per rule. `stated` holds
/// the rule's parameters the list names, by their own names.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    declaration(&synthesised(stated.clone())).map(|_| ())
}

/// A three-valued judgement of an interval against a tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holds {
    Yes,
    No,
    Unknown,
}

impl Holds {
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }
    pub(crate) fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            (Self::No, Self::No) => Self::No,
            _ => Self::Unknown,
        }
    }
    #[cfg_attr(not(feature = "parity-reference"), allow(dead_code))]
    pub(crate) fn not(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// A measured length as a message reads it.
pub(crate) fn described(interval: Option<axioval_engine::LengthInterval>) -> String {
    interval.map_or_else(
        || "unmeasured".to_owned(),
        |interval| {
            if interval.is_exact() {
                format!("{:.4} m", interval.lower_metres())
            } else {
                format!(
                    "{:.4} to {:.4} m",
                    interval.lower_metres(),
                    interval.upper_metres()
                )
            }
        },
    )
}

/// A measured shared volume as a message reads it.
pub(crate) fn described_volume(shared: Option<axioval_engine::VolumeInterval>) -> String {
    shared.map_or_else(
        || "an unmeasured volume".to_owned(),
        |shared| {
            if shared.is_exact() {
                format!("{:.6} m³", shared.lower_cubic_metres())
            } else {
                format!(
                    "{:.6} to {:.6} m³",
                    shared.lower_cubic_metres(),
                    shared.upper_cubic_metres()
                )
            }
        },
    )
}

/// The class a reported pair falls into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    Duplicate,
    Containment,
    Intersection,
    Clearance,
    /// A pair no clash matrix cell covers.
    #[cfg_attr(not(feature = "parity-reference"), allow(dead_code))]
    Unmatched,
}

impl Class {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Duplicate => "duplicate",
            Self::Containment => "containment",
            Self::Intersection => "intersection",
            Self::Clearance => "clearance",
            Self::Unmatched => "unmatched",
        }
    }
}

type Reached = Result<BTreeSet<ObjectId>, Unavailable>;

/// Whether pairs share a relationship target or a presentation layer, with
/// every walk and layer read cached per object.
pub(crate) struct Exclusions<'r> {
    context: &'r RuleContext<'r>,
    paths: Vec<Traversal>,
    /// Reached targets meet when they state the same value of this property.
    target_property: Option<PropertyRef<'r>>,
    same_layer: bool,
    everything: Vec<&'r Object>,
    reached: BTreeMap<(usize, ObjectId), Reached>,
    layers: BTreeMap<ObjectId, Result<BTreeSet<String>, Unavailable>>,
    /// A target's value of `target_property`: its key and how it reads,
    /// `None` when it states none.
    labels: BTreeMap<ObjectId, Label>,
}

type Label = Result<Option<(String, String)>, Unavailable>;

/// Whether two sets of reached targets meet by a stated value.
enum Labelled {
    Same(String),
    Different,
    Undecided(Unavailable),
}

impl<'r> Exclusions<'r> {
    pub(crate) fn new(
        context: &'r RuleContext<'r>,
        paths: &'r [Vec<String>],
        target_property: Option<PropertyRef<'r>>,
        same_layer: bool,
    ) -> Result<Self, Unavailable> {
        if target_property.is_some() && paths.is_empty() {
            return Err(invalid(
                "`exclude_target_property` compares the targets exclusion paths reach, \
                 but no exclusion path is declared",
            ));
        }
        Ok(Self {
            context,
            paths: paths
                .iter()
                .map(|path| Traversal::path(path))
                .collect::<Result<_, _>>()?,
            target_property,
            same_layer,
            everything: context.project.objects().collect(),
            reached: BTreeMap::new(),
            layers: BTreeMap::new(),
            labels: BTreeMap::new(),
        })
    }

    /// The value a reached target states for `property`.
    fn label(&mut self, property: PropertyRef<'_>, target: &ObjectId) -> &Label {
        let context = self.context;
        self.labels.entry(target.clone()).or_insert_with(|| {
            let Some(object) = context.project.object(target) else {
                return Err((
                    NotEvaluatedReason::InvalidEvidence,
                    format!("the reached target {target} is not in the project"),
                ));
            };
            let resolved = resolve(context, object, property)?;
            Ok(match resolved.value() {
                Some(value) if !undefined(Some(value)) => {
                    Some((value_key(value, false, true), display(Some(value))))
                }
                _ => None,
            })
        })
    }

    /// Whether a target reached from one member states the same value of
    /// `property` as a target reached from the other. The members are not
    /// targets of their own: two walls of one name are not one system.
    fn labelled(
        &mut self,
        property: PropertyRef<'_>,
        (a, from_a): (&ObjectId, &BTreeSet<ObjectId>),
        (b, from_b): (&ObjectId, &BTreeSet<ObjectId>),
    ) -> Labelled {
        let mut side = |member: &ObjectId, reached: &BTreeSet<ObjectId>| {
            let mut known = BTreeMap::new();
            let mut unknown = None;
            let mut any = false;
            for target in reached.iter().filter(|target| *target != member) {
                any = true;
                match self.label(property, target) {
                    Ok(Some((key, shown))) => {
                        known.insert(key.clone(), shown.clone());
                    }
                    Ok(None) => {}
                    Err(why) => {
                        unknown.get_or_insert_with(|| why.clone());
                    }
                }
            }
            (known, unknown, any)
        };
        let (known_a, unknown_a, any_a) = side(a, from_a);
        let (known_b, unknown_b, any_b) = side(b, from_b);
        if let Some((_, shown)) = known_a.iter().find(|(key, _)| known_b.contains_key(*key)) {
            return Labelled::Same(shown.clone());
        }
        // An unread value matters only with a target on the other side.
        match (unknown_a, unknown_b) {
            (Some(why), _) if any_b => Labelled::Undecided(why),
            (_, Some(why)) if any_a => Labelled::Undecided(why),
            _ => Labelled::Different,
        }
    }

    fn reached(&mut self, path: usize, object: &ObjectId) -> &Reached {
        let Self {
            context,
            paths,
            everything,
            reached,
            ..
        } = self;
        reached.entry((path, object.clone())).or_insert_with(|| {
            paths[path]
                .related(context, object, everything)
                .map(|(found, _)| found.into_iter().collect())
        })
    }

    fn layers(&mut self, object: &ObjectId) -> &Result<BTreeSet<String>, Unavailable> {
        let context = self.context;
        self.layers.entry(object.clone()).or_insert_with(|| {
            let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
                return Err((
                    NotEvaluatedReason::MissingService,
                    "property-resolution service is not registered".into(),
                ));
            };
            // The presentation layer is engine vocabulary, not a package
            // concept: it is asked for by its own name in every source.
            let request = PropertyRequest::try_new(
                object.clone(),
                Some(PRESENTATION_SET.to_owned()),
                PRESENTATION_LAYER,
            )
            .map_err(|error| invalid(error.to_string()))?;
            let resolved = match service.resolve(&request).map_err(property_error)? {
                PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
                PropertyResolution::Absent(_) => None,
            };
            match resolved.as_ref() {
                None => Ok(BTreeSet::new()),
                Some(PropertyValue::List(values)) => values
                    .iter()
                    .map(|value| match value {
                        PropertyValue::String(layer) => Ok(layer.clone()),
                        _ => Err((
                            NotEvaluatedReason::InvalidEvidence,
                            format!("a presentation layer of {object} is not text"),
                        )),
                    })
                    .collect(),
                Some(_) => Err((
                    NotEvaluatedReason::InvalidEvidence,
                    format!("the presentation layers of {object} are not a list"),
                )),
            }
        })
    }

    /// Why the pair is excluded, `None` when it is not, or why that cannot
    /// be decided.
    pub(crate) fn excluded(
        &mut self,
        a: &ObjectId,
        b: &ObjectId,
    ) -> Result<Option<String>, Unavailable> {
        let mut undecided = None;
        for path in 0..self.paths.len() {
            let from_a = self.reached(path, a).clone();
            let from_b = self.reached(path, b).clone();
            // Each object counts as reaching itself, so a path from one
            // object to the other excludes the pair as well.
            let shared = match (&from_a, &from_b) {
                (Ok(from_a), _) if from_a.contains(b) => Some(true),
                (_, Ok(from_b)) if from_b.contains(a) => Some(true),
                (Ok(from_a), Ok(from_b)) => Some(!from_a.is_disjoint(from_b)),
                _ => None,
            };
            match shared {
                Some(true) => {
                    return Ok(Some(format!(
                        "they share a target through {}",
                        self.paths[path].relationship
                    )));
                }
                Some(false) => {
                    let (Some(property), Ok(from_a), Ok(from_b)) =
                        (self.target_property, &from_a, &from_b)
                    else {
                        continue;
                    };
                    match self.labelled(property, (a, from_a), (b, from_b)) {
                        Labelled::Same(value) => {
                            return Ok(Some(format!(
                                "they reach targets stating {property} {value} through {}",
                                self.paths[path].relationship
                            )));
                        }
                        Labelled::Different => {}
                        Labelled::Undecided((reason, message)) => {
                            undecided.get_or_insert((
                                reason,
                                format!(
                                    "whether they reach targets stating the same {property} through {} cannot be decided: {message}",
                                    self.paths[path].relationship
                                ),
                            ));
                        }
                    }
                }
                None => {
                    let (reason, message) = from_a
                        .err()
                        .or(from_b.err())
                        .unwrap_or_else(|| unreachable!("an undecided path has a failed walk"));
                    undecided.get_or_insert((
                        reason,
                        format!(
                            "whether they share a target through {} cannot be decided: {message}",
                            self.paths[path].relationship
                        ),
                    ));
                }
            }
        }
        // Layers are named per model: one name in two models is a
        // coincidence, not a shared layer.
        if self.same_layer && a.source == b.source {
            let (on_a, on_b) = (self.layers(a).clone(), self.layers(b).clone());
            match (on_a, on_b) {
                (Ok(on_a), Ok(on_b)) => {
                    if let Some(layer) = on_a.intersection(&on_b).next() {
                        return Ok(Some(format!("they share the presentation layer {layer}")));
                    }
                }
                (Err((reason, message)), _) | (_, Err((reason, message))) => {
                    undecided.get_or_insert((
                        reason,
                        format!(
                            "whether they share a presentation layer cannot be decided: {message}"
                        ),
                    ));
                }
            }
        }
        match undecided {
            Some(unavailable) => Err(unavailable),
            None => Ok(None),
        }
    }
}

/// What an undecided exclusion leaves a pair that would be reported: open,
/// with its reason and words. A fact the source records for nothing is
/// about the source, not the pair: its words name no object, so the
/// runtime reports it once per source.
pub(crate) fn excluded_words(
    (reason, message): &Unavailable,
    counterpart: &ObjectId,
) -> (NotEvaluatedReason, String) {
    if *reason == NotEvaluatedReason::NotRecorded {
        (
            NotEvaluatedReason::NotRecorded,
            format!("a clash may be excluded: {message}"),
        )
    } else {
        (
            reason.clone(),
            format!("the pair with {counterpart} may be excluded: {message}"),
        )
    }
}

/// The parameters of a profile, in `clash`'s order.
pub(crate) fn profile_columns() -> impl Iterator<Item = (&'static str, bool)> {
    PROFILE_NUMBERS
        .iter()
        .map(|name| (*name, true))
        .chain(PROFILE_SWITCHES.iter().map(|name| (*name, false)))
}
