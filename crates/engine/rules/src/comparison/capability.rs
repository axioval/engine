//! `model-comparison`: two models of one run compared as a rule.
//!
//! The two models are sources of the session, named by the discipline each
//! declares (`--model a.ifc:base --model b.ifc:revised`), so a comparison
//! runs in a ruleset beside other rules and its findings travel in the same
//! report. The rule's selector restricts the objects compared, on both
//! sides unless `revised_selector` restricts the revised model; an object
//! the selector cannot decide is compared all the same, and whatever depends
//! on it alone is not evaluated rather than reported. With both models in
//! one session, objects without stable identities can be matched by their
//! bodies (`geometry`, `placement`, `overlap`) or through a related object
//! (`related`: a door through its opening).

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, SourceDisciplines, SourceMetadataIndex, TableColumn,
};
use axioval_ir::contract::SourceField;
use axioval_ir::contract::{ColumnKind, Selector};
use axioval_ir::{Object, ObjectId, SourceId};

use super::{
    ComparedObject, ComparedProperty, ComparisonRequest, ComparisonTolerance, Matcher, Naming,
    ObjectChange, Revision, compare,
};
use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, PropertyRef, Unavailable, invalid};

/// Compares the model of discipline `base` with the model of discipline
/// `revised`, object by object, and reports what was added, removed and
/// changed as findings of the rule.
///
/// Objects are matched by `identity_scheme`, then by `identity_property`
/// (read as `revised_identity_property` on the revised model when given), or
/// in the order `match_by` names them, which may add the body matchers
/// `geometry`, `placement` and `overlap` (within `match_length_tolerance`,
/// `match_angle_tolerance` and `minimum_overlap_ratio`) and `related`
/// (along `match_path`). With `compare_timestamps`, a revised model written
/// before its base is an error finding. Compared are the kind, the
/// classifications, the carried properties and relationships, the named
/// `properties`, the `property_sets` (or, with `all_property_sets`, every
/// set) listed through property enumeration, and, when asked, the related
/// objects per relationship kind (`compare_relationships`), placement,
/// geometry and coordinate systems within `length_tolerance` (metres) and
/// `angle_tolerance` (degrees). Geometry compares bounds, or with
/// `geometry: mesh` the certified distance between the two surfaces within
/// `tolerance_metres`.
pub struct CompareModels;

const PROPERTIES: &[TableColumn] = &[
    TableColumn::optional("property_set", ColumnKind::String),
    TableColumn::required("property", ColumnKind::String),
];

const PROPERTY_SETS: &[TableColumn] = &[TableColumn::required("property_set", ColumnKind::String)];

/// What a rule declares, read and checked.
struct Declaration<'a> {
    base: &'a str,
    revised: &'a str,
    revised_selector: Option<&'a Selector>,
    request: ComparisonRequest,
}

fn property(reference: PropertyRef<'_>) -> Result<ComparedProperty, Unavailable> {
    ComparedProperty::new(reference.set, reference.name)
        .map_err(|error| invalid(format!("`{reference}`: {error}")))
}

impl<'a> Declaration<'a> {
    fn read(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let base = parameters.required_string("base")?;
        let revised = parameters.required_string("revised")?;
        if base == revised {
            return Err(invalid(
                "`base` and `revised` name the same discipline; a model is not compared with itself",
            ));
        }
        let request = ComparisonRequest::matching(Self::matchers(&parameters)?)
            .map_err(|error| invalid(error.to_string()))?;
        let request = Self::facets(&parameters, request)?;
        Ok(Self {
            base,
            revised,
            revised_selector: parameters.selector("revised_selector")?,
            request,
        })
    }

    fn matchers(parameters: &Parameters<'a>) -> Result<Vec<Matcher>, Unavailable> {
        let scheme = parameters.string("identity_scheme")?;
        let identity = parameters.property("identity_property")?;
        let revised_identity = parameters.property("revised_identity_property")?;
        let property_matcher = match (identity, revised_identity) {
            (Some(base), revised) => {
                let base = property(base)?;
                let revised = revised
                    .map(property)
                    .transpose()?
                    .unwrap_or_else(|| base.clone());
                Some(Matcher::Property { base, revised })
            }
            (None, Some(_)) => {
                return Err(invalid(
                    "`revised_identity_property` needs `identity_property`",
                ));
            }
            (None, None) => None,
        };
        let scheme_matcher = scheme.map(|scheme| Matcher::Scheme(scheme.to_owned()));
        let Some(order) = parameters.strings("match_by")? else {
            return Ok(scheme_matcher.into_iter().chain(property_matcher).collect());
        };
        let length = parameters
            .number("match_length_tolerance")?
            .unwrap_or(0.001);
        let angle = parameters
            .number("match_angle_tolerance")?
            .unwrap_or(0.01)
            .to_radians();
        let tolerance = ComparisonTolerance::try_new(length, angle)
            .map_err(|error| invalid(format!("`match_*_tolerance`: {error}")))?;
        let mut matchers = Vec::new();
        for name in order {
            let matcher = match name.as_str() {
                "identity" => scheme_matcher.clone().ok_or_else(|| {
                    invalid("`match_by` names `identity`, which needs `identity_scheme`")
                })?,
                "property" => property_matcher.clone().ok_or_else(|| {
                    invalid("`match_by` names `property`, which needs `identity_property`")
                })?,
                "geometry" => Matcher::Geometry {
                    tolerance_metres: length,
                },
                "placement" => Matcher::Placement { tolerance },
                "overlap" => Matcher::Overlap {
                    minimum_ratio: parameters.number("minimum_overlap_ratio")?.ok_or_else(
                        || {
                            invalid(
                                "`match_by` names `overlap`, which needs `minimum_overlap_ratio`",
                            )
                        },
                    )?,
                },
                "related" => Matcher::Related {
                    path: parameters
                        .strings("match_path")?
                        .ok_or_else(|| {
                            invalid("`match_by` names `related`, which needs `match_path`")
                        })?
                        .to_vec(),
                    tolerance_metres: length,
                },
                other => {
                    return Err(invalid(format!(
                        "`match_by` names `{other}`; the matchers are `identity`, `property`, `geometry`, `placement`, `overlap` and `related`"
                    )));
                }
            };
            if matchers.contains(&matcher) {
                return Err(invalid(format!("`match_by` names `{name}` twice")));
            }
            matchers.push(matcher);
        }
        Ok(matchers)
    }

    fn facets(
        parameters: &Parameters<'a>,
        mut request: ComparisonRequest,
    ) -> Result<ComparisonRequest, Unavailable> {
        for row in parameters.table("properties")?.unwrap_or_default() {
            let name = row
                .text("property")?
                .ok_or_else(|| invalid("a `properties` row names no property"))?;
            request = request
                .with_property(row.text("property_set")?, name)
                .map_err(|error| invalid(error.to_string()))?;
        }
        for row in parameters.table("property_sets")?.unwrap_or_default() {
            let set = row
                .text("property_set")?
                .ok_or_else(|| invalid("a `property_sets` row names no set"))?;
            request = request
                .with_property_set(set)
                .map_err(|error| invalid(error.to_string()))?;
        }
        if parameters.boolean("all_property_sets")?.unwrap_or(false) {
            request = request.with_all_property_sets();
        }
        let tolerance = ComparisonTolerance::try_new(
            parameters.number("length_tolerance")?.unwrap_or(0.0),
            parameters
                .number("angle_tolerance")?
                .unwrap_or(0.0)
                .to_radians(),
        )
        .map_err(|error| invalid(error.to_string()))?;
        if parameters.boolean("compare_placement")?.unwrap_or(false) {
            request = request.with_placement(tolerance);
        }
        request = Self::geometry(parameters, request, tolerance)?;
        if parameters
            .boolean("compare_coordinate_systems")?
            .unwrap_or(false)
        {
            request = request.with_coordinate_systems(tolerance);
        }
        if parameters
            .boolean("compare_relationships")?
            .unwrap_or(false)
        {
            request = request.with_relationships();
        }
        if parameters.boolean("compare_timestamps")?.unwrap_or(false) {
            request = request.with_timestamps();
        }
        Ok(request)
    }

    /// The geometry facet: `compare_geometry` compares bounds within
    /// `length_tolerance`; `geometry` names the mode (`bounds`, or `mesh`
    /// within `tolerance_metres`) and compares geometry by itself.
    fn geometry(
        parameters: &Parameters<'a>,
        request: ComparisonRequest,
        tolerance: ComparisonTolerance,
    ) -> Result<ComparisonRequest, Unavailable> {
        let compare = parameters.boolean("compare_geometry")?;
        let mesh_tolerance = parameters.number("tolerance_metres")?;
        let Some(mode) = parameters.string("geometry")? else {
            if mesh_tolerance.is_some() {
                return Err(invalid("`tolerance_metres` needs `geometry: mesh`"));
            }
            return Ok(if compare.unwrap_or(false) {
                request.with_geometry(tolerance)
            } else {
                request
            });
        };
        if compare == Some(false) {
            return Err(invalid(format!(
                "`geometry: {mode}` compares geometry, but `compare_geometry` is false"
            )));
        }
        match mode {
            "bounds" if mesh_tolerance.is_some() => {
                Err(invalid("`tolerance_metres` needs `geometry: mesh`"))
            }
            "bounds" => Ok(request.with_geometry(tolerance)),
            "mesh" => {
                let length = mesh_tolerance
                    .ok_or_else(|| invalid("`geometry: mesh` needs `tolerance_metres`"))?;
                let tolerance = ComparisonTolerance::try_new(length, tolerance.angle_radians())
                    .map_err(|error| invalid(format!("`tolerance_metres`: {error}")))?;
                Ok(request.with_mesh_geometry(tolerance))
            }
            other => Err(invalid(format!(
                "`geometry` is `{other}`; the modes are `bounds` and `mesh`"
            ))),
        }
    }

    /// The one source of each named discipline.
    fn sources(&self, context: &RuleContext<'_>) -> Result<(SourceId, SourceId), Unavailable> {
        let Some(disciplines) = context.services.get::<SourceDisciplines>() else {
            return Err((
                NotEvaluatedReason::MissingService,
                "source disciplines are not available outside an evidence session".into(),
            ));
        };
        let mut found: [Vec<SourceId>; 2] = [Vec::new(), Vec::new()];
        let mut undeclared = 0_usize;
        for source in crate::support::sources(context) {
            match disciplines.of(&source).map(axioval_ir::Discipline::as_str) {
                Some(discipline) if discipline == self.base => found[0].push(source),
                Some(discipline) if discipline == self.revised => found[1].push(source),
                Some(_) => {}
                None => undeclared += 1,
            }
        }
        let [base, revised] = found;
        let one =
            |sources: Vec<SourceId>, discipline: &str| match <[SourceId; 1]>::try_from(sources) {
                Ok([source]) => Ok(source),
                Err(sources) if sources.is_empty() && undeclared > 0 => Err((
                    NotEvaluatedReason::NotRecorded,
                    format!(
                        "no model declares discipline `{discipline}`, and {undeclared} declare none"
                    ),
                )),
                Err(sources) if sources.is_empty() => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("no model of discipline `{discipline}` is checked"),
                )),
                Err(sources) => Err((
                    NotEvaluatedReason::InvalidEvidence,
                    format!(
                        "{} models declare discipline `{discipline}`; the comparison needs one",
                        sources.len()
                    ),
                )),
            };
        Ok((one(base, self.base)?, one(revised, self.revised)?))
    }
}

/// The objects of `source` the rule's selector picks or cannot decide.
struct Selected<'a> {
    revision: Revision<'a>,
    undecided: BTreeMap<ObjectId, (NotEvaluatedReason, String)>,
}

fn select<'a>(context: &RuleContext<'a>, selector: &Selector, source: &SourceId) -> Selected<'a> {
    let objects: Vec<&'a Object> = context
        .project
        .objects()
        .filter(|object| &object.id.source == source)
        .collect();
    let timestamps = context
        .services
        .get::<SourceMetadataIndex>()
        .and_then(|metadata| metadata.values(source, SourceField::Timestamp))
        .map(<[String]>::to_vec);
    let mut candidates = Vec::new();
    let mut undecided = BTreeMap::new();
    for object in &objects {
        match selector_matches(context, selector, object, &mut Vec::new()) {
            Selection::Match => candidates.push(*object),
            Selection::NoMatch => {}
            Selection::NotEvaluated(reason, message) => {
                candidates.push(*object);
                undecided.insert(object.id.clone(), (reason, message));
            }
        }
    }
    Selected {
        revision: Revision {
            context: RuleContext {
                project: context.project,
                services: context.services,
            },
            candidates,
            objects,
            timestamps: BTreeMap::from([(source.clone(), timestamps)]),
        },
        undecided,
    }
}

impl RuleCapability for CompareModels {
    fn id(&self) -> &'static str {
        "axioval:capability.model-comparison"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("base", ParameterType::String),
            ParameterDescriptor::required("revised", ParameterType::String),
            ParameterDescriptor::optional("identity_scheme", ParameterType::String),
            ParameterDescriptor::optional("identity_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional(
                "revised_identity_property",
                ParameterType::PropertyReference,
            ),
            ParameterDescriptor::optional("match_by", ParameterType::StringList),
            ParameterDescriptor::optional("properties", ParameterType::Table(PROPERTIES)),
            ParameterDescriptor::optional("property_sets", ParameterType::Table(PROPERTY_SETS)),
            ParameterDescriptor::optional("all_property_sets", ParameterType::Boolean),
            ParameterDescriptor::optional("compare_placement", ParameterType::Boolean),
            ParameterDescriptor::optional("compare_geometry", ParameterType::Boolean),
            ParameterDescriptor::optional("compare_coordinate_systems", ParameterType::Boolean),
            ParameterDescriptor::optional("length_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("angle_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("revised_selector", ParameterType::Selector),
            ParameterDescriptor::optional("match_length_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("match_angle_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("minimum_overlap_ratio", ParameterType::Number),
            ParameterDescriptor::optional("match_path", ParameterType::StringList),
            ParameterDescriptor::optional("compare_timestamps", ParameterType::Boolean),
            ParameterDescriptor::optional("compare_relationships", ParameterType::Boolean),
            ParameterDescriptor::optional("geometry", ParameterType::String),
            ParameterDescriptor::optional("tolerance_metres", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = Declaration::read(rule).and_then(|declaration| {
            let sources = declaration.sources(context)?;
            Ok((declaration, sources))
        });
        let (declaration, (base_source, revised_source)) = match declared {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("model-comparison: {message}"),
                );
            }
        };
        let base = select(context, &rule.selector, &base_source);
        let revised = select(
            context,
            declaration.revised_selector.unwrap_or(&rule.selector),
            &revised_source,
        );
        let pairs = if declaration.request.compares_sources() {
            vec![(base_source, revised_source)]
        } else {
            Vec::new()
        };
        let comparison = compare(
            &base.revision,
            &revised.revision,
            &declaration.request,
            (pairs, Vec::new()),
        );
        let undecided = |object: &ObjectId| {
            base.undecided
                .get(object)
                .or_else(|| revised.undecided.get(object))
        };
        // A change is reported only when a compared object the selector
        // surely picks is part of it.
        let out_of_scope = |compared: &ComparedObject| {
            let (subject, partner) = match &compared.change {
                ObjectChange::Added { revised } => (revised, None),
                ObjectChange::Removed { base } => (base, None),
                ObjectChange::Matched { base, revised, .. } => (revised, Some(base)),
            };
            let (reason, message) = undecided(subject)?;
            if partner.is_some_and(|partner| undecided(partner).is_none()) {
                return None;
            }
            Some((
                subject.clone(),
                reason.clone(),
                format!("the selector cannot decide whether it is compared: {message}"),
            ))
        };
        let report = comparison.project(
            Naming::Prefixed(&rule.id),
            &crate::pairs::severity(rule),
            out_of_scope,
        );
        let mut evaluation = CapabilityEvaluation::default();
        for finding in report.findings {
            evaluation.push_finding(finding);
        }
        for outcome in report.not_evaluated {
            evaluation.push_not_evaluated_about(outcome.scope, outcome.reason, outcome.message);
        }
        evaluation
    }
}
