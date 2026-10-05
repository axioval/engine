//! Properties the engine derives instead of reading them from a source: the
//! class names a ruleset's classifications assign
//! ([`axioval_ir::CLASSIFICATION_SET`]) and values measured from geometry
//! ([`axioval_ir::MEASURED_SET`], see [`crate::measured`]).
//!
//! The runtime answers them through the one property-resolution handle, so
//! every selector and capability reads a derived property exactly as it
//! reads a stated one. What cannot be derived is an error of the request,
//! never an absence.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_ir::contract::{
    ClassTree, ClassificationDefinition, ClassificationMode, ClassificationProperty,
};
use axioval_ir::{
    CLASSIFICATION_SET, Evidence, GROUP_SET, MEASURED_SET, NotEvaluatedReason, ObjectId, Property,
    PropertyValue, VALUE_SET,
};

use crate::groupings::DerivedGroups;
use crate::measured::Measures;
use crate::properties::{
    CompletePropertyAbsenceEvidence, PropertyEnumeration, PropertyEnumerationRequest,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty,
};
use crate::session::SourceSnapshot;
use crate::values::{DerivedValues, ValueExpressions};
use crate::{OutcomeRefiner, RuleContext, SelectorVerdict};

/// What one classification assigned one object.
#[derive(Clone, Debug, PartialEq)]
pub enum ClassOutcome {
    /// The class names of the matching rows, distinct in row order (one
    /// for a first-match classification), with the rows' indices.
    Classified {
        classes: Vec<String>,
        rows: Vec<usize>,
    },
    /// Every row surely does not match.
    Unclassified,
    /// A row that could decide the class could not be decided, and why.
    Undecided(NotEvaluatedReason, String),
}

/// Every object's classes under every classification of a run.
#[derive(Clone, Debug, Default)]
pub struct Classifications {
    modes: BTreeMap<String, ClassificationMode>,
    trees: BTreeMap<String, ClassTree>,
    outcomes: BTreeMap<String, BTreeMap<ObjectId, ClassOutcome>>,
}

impl Classifications {
    /// Whether the run declares the classification `id`.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.outcomes.contains_key(id)
    }

    /// What the classification `id` assigned `object`; `None` for an
    /// undeclared classification or an object outside the project.
    #[must_use]
    pub fn outcome(&self, id: &str, object: &ObjectId) -> Option<&ClassOutcome> {
        self.outcomes.get(id)?.get(object)
    }

    /// The classes of the classification `id`; `None` when undeclared.
    #[must_use]
    pub fn tree(&self, id: &str) -> Option<&ClassTree> {
        self.trees.get(id)
    }

    /// Whether the classification `id` assigned `object` the class `class`
    /// or, with `include_descendants`, a class within it; an all-match
    /// classification when any class it assigned does. An unclassified
    /// object is not selected. An error gives the reason an object cannot
    /// be decided: its class could not be derived, or the classification or
    /// the object is not part of this run.
    pub fn selects(
        &self,
        id: &str,
        class: &str,
        include_descendants: bool,
        object: &ObjectId,
    ) -> Result<bool, (NotEvaluatedReason, String)> {
        let (Some(tree), Some(outcome)) = (self.tree(id), self.outcome(id, object)) else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("classification `{id}` did not classify {object} in this run"),
            ));
        };
        match outcome {
            ClassOutcome::Classified { classes, .. } => Ok(classes.iter().any(|assigned| {
                assigned == class || (include_descendants && tree.is_within(assigned, class))
            })),
            ClassOutcome::Unclassified => Ok(false),
            ClassOutcome::Undecided(reason, message) => Err((reason.clone(), message.clone())),
        }
    }

    /// Classifies every object of the context's project by `definition`,
    /// its rows evaluated by `refiner`, and adds the result.
    ///
    /// A definition whose classes do not form a valid tree (which
    /// compilation refuses) leaves every object undecided.
    pub(crate) fn classify(
        &mut self,
        refiner: &dyn OutcomeRefiner,
        context: &RuleContext<'_>,
        definition: &ClassificationDefinition,
    ) {
        let tree = ClassTree::of(definition);
        let outcomes = context
            .project
            .objects()
            .map(|object| {
                let outcome = match &tree {
                    Ok(_) => {
                        let rows = definition
                            .rows
                            .iter()
                            .map(|row| refiner.evaluate_selector(context, &row.selector, object));
                        classify_one(definition, rows)
                    }
                    Err(error) => ClassOutcome::Undecided(
                        NotEvaluatedReason::InvalidEvidence,
                        format!("classification `{}` is invalid: {error}", definition.id),
                    ),
                };
                (object.id.clone(), outcome)
            })
            .collect();
        self.modes.insert(definition.id.clone(), definition.mode);
        if let Ok(tree) = tree {
            self.trees.insert(definition.id.clone(), tree);
        }
        self.outcomes.insert(definition.id.clone(), outcomes);
    }

    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let name = request.property();
        let read = ClassificationProperty::parse(name)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let id = read.classification;
        let object = request.object_id();
        let Some(outcome) = self.outcome(id, object) else {
            return Err(if self.contains(id) {
                PropertyResolutionError::Unavailable(format!(
                    "{object} is not an object of this run"
                ))
            } else {
                PropertyResolutionError::InvalidRequest
            });
        };
        let evidence = |locator: String| Evidence::exact(object.source.clone(), locator);
        match outcome {
            ClassOutcome::Classified { classes, rows } => {
                // The classes read: the assigned ones, or those at the
                // level asked for, distinct in row order.
                let mut read_classes: Vec<&str> = Vec::new();
                for class in classes {
                    let read_class = match read.level {
                        None => Some(class.as_str()),
                        Some(level) => self
                            .tree(id)
                            .filter(|tree| tree.is_hierarchical())
                            .ok_or(PropertyResolutionError::InvalidRequest)?
                            .at_level(class, level),
                    };
                    if let Some(read_class) = read_class
                        && !read_classes.contains(&read_class)
                    {
                        read_classes.push(read_class);
                    }
                }
                let rows = rows
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                if read_classes.is_empty() {
                    // Every assigned class lies above the level: the object
                    // surely has no class there.
                    return Ok(PropertyResolution::Absent(
                        CompletePropertyAbsenceEvidence::try_new(
                            request.clone(),
                            evidence(format!(
                                "{CLASSIFICATION_SET}/{name}#above-level;rows={rows}"
                            )),
                        )?,
                    ));
                }
                let value = match self.modes[id] {
                    ClassificationMode::FirstMatch => {
                        PropertyValue::String(read_classes[0].to_owned())
                    }
                    ClassificationMode::AllMatch => PropertyValue::List(
                        read_classes
                            .iter()
                            .map(|class| PropertyValue::String((*class).to_owned()))
                            .collect(),
                    ),
                };
                let property = Property::new(CLASSIFICATION_SET, name, value)
                    .map_err(|_| PropertyResolutionError::InvalidRequest)?
                    .with_evidence(evidence(format!("{CLASSIFICATION_SET}/{name}#rows={rows}")));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            ClassOutcome::Unclassified => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    evidence(format!("{CLASSIFICATION_SET}/{name}#no-row")),
                )?,
            )),
            ClassOutcome::Undecided(reason, message) => Err(match reason {
                NotEvaluatedReason::NotRecorded => {
                    PropertyResolutionError::NotRecorded(message.clone())
                }
                NotEvaluatedReason::MissingService | NotEvaluatedReason::BackendUnavailable => {
                    PropertyResolutionError::Unavailable(message.clone())
                }
                _ => PropertyResolutionError::Incomplete(message.clone()),
            }),
        }
    }
}

/// One object's classes from its rows' verdicts, in row order.
///
/// First match: the first matching row decides, once every row before it
/// surely does not match; an undecided row before it leaves the object
/// undecided. All match: every row must be decided.
fn classify_one(
    definition: &ClassificationDefinition,
    rows: impl Iterator<Item = SelectorVerdict>,
) -> ClassOutcome {
    let first = definition.mode == ClassificationMode::FirstMatch;
    let mut classes: Vec<String> = Vec::new();
    let mut matched = Vec::new();
    for (index, verdict) in rows.enumerate() {
        match verdict {
            SelectorVerdict::Match(_) => {
                let class = &definition.rows[index].class;
                if !classes.contains(class) {
                    classes.push(class.clone());
                }
                matched.push(index);
                if first {
                    break;
                }
            }
            SelectorVerdict::NoMatch(_) => {}
            SelectorVerdict::Undecided(reason, message) => {
                return ClassOutcome::Undecided(
                    reason,
                    format!(
                        "row {index} of classification `{}` cannot be decided: {message}",
                        definition.id
                    ),
                );
            }
        }
    }
    if classes.is_empty() {
        ClassOutcome::Unclassified
    } else {
        ClassOutcome::Classified {
            classes,
            rows: matched,
        }
    }
}

/// A field of [`axioval_ir::SOURCE_SET`].
#[derive(Clone, Copy)]
pub(crate) enum SourceFact {
    /// The declared discipline.
    Discipline,
    /// One metadata field.
    Metadata(axioval_ir::contract::SourceField),
}

/// What a field of [`axioval_ir::SOURCE_SET`] reads; `None` for any other
/// name.
pub(crate) fn source_field(name: &str) -> Option<SourceFact> {
    use axioval_ir::contract::SourceField;
    if name == axioval_ir::SOURCE_DISCIPLINE {
        return Some(SourceFact::Discipline);
    }
    [
        SourceField::FileName,
        SourceField::Application,
        SourceField::Schema,
        SourceField::Project,
        SourceField::Timestamp,
    ]
    .into_iter()
    .find(|field| field.as_str() == name)
    .map(SourceFact::Metadata)
}

/// What the session knows about each source, answering
/// [`axioval_ir::SOURCE_SET`].
#[derive(Clone, Default)]
pub(crate) struct SourceFacts {
    disciplines: Option<crate::SourceDisciplines>,
    metadata: Option<crate::SourceMetadataIndex>,
}

impl SourceFacts {
    pub(crate) fn of(services: &crate::ServiceRegistry) -> Self {
        Self {
            disciplines: services.get::<crate::SourceDisciplines>().cloned(),
            metadata: services.get::<crate::SourceMetadataIndex>().cloned(),
        }
    }

    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let source = &request.object_id().source;
        let field =
            source_field(request.property()).ok_or(PropertyResolutionError::InvalidRequest)?;
        let evidence = || {
            axioval_ir::Evidence::exact(
                source.clone(),
                format!("{}/{}", axioval_ir::SOURCE_SET, request.property()),
            )
        };
        let values: Vec<String> = match field {
            SourceFact::Discipline => {
                let disciplines = self.disciplines.as_ref().ok_or_else(|| {
                    PropertyResolutionError::MissingService(
                        "source disciplines are not available outside an evidence session".into(),
                    )
                })?;
                disciplines
                    .of(source)
                    .map(|discipline| vec![discipline.as_str().to_owned()])
                    .unwrap_or_default()
            }
            SourceFact::Metadata(field) => self
                .metadata
                .as_ref()
                .ok_or_else(|| {
                    PropertyResolutionError::MissingService(
                        "source metadata is not available outside an evidence session".into(),
                    )
                })?
                .values(source, field)
                .ok_or_else(|| {
                    PropertyResolutionError::NotRecorded(format!(
                        "source `{source}` does not record its {}",
                        field.as_str()
                    ))
                })?
                .to_vec(),
        };
        let value = match &values[..] {
            [] => {
                return Ok(PropertyResolution::Absent(
                    CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence())?,
                ));
            }
            [one] => PropertyValue::String(one.clone()),
            several => {
                PropertyValue::List(several.iter().cloned().map(PropertyValue::String).collect())
            }
        };
        let property = Property::new(axioval_ir::SOURCE_SET, request.property(), value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?
            .with_evidence(evidence());
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
    }
}

/// The run's property resolver: derived sets answered by the engine, every
/// other request by the host's resolver.
pub(crate) struct DerivedProperties {
    pub(crate) inner: Option<PropertyResolutionServiceHandle>,
    pub(crate) measures: Measures,
    pub(crate) sources: SourceFacts,
    pub(crate) classifications: Arc<Classifications>,
    pub(crate) groups: Arc<DerivedGroups>,
    pub(crate) values: Arc<DerivedValues>,
    pub(crate) snapshots: Vec<SourceSnapshot>,
}

impl PropertyResolutionService for DerivedProperties {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if request.property_set() == Some(GROUP_SET)
            || (request.property_set() != Some(MEASURED_SET)
                && self.groups.group(request.object_id()).is_some())
        {
            return self.groups.resolve(request);
        }
        if request.property_set() == Some(CLASSIFICATION_SET) {
            return self.classifications.resolve(request);
        }
        if request.property_set() == Some(VALUE_SET) {
            return self
                .values
                .resolve(request, &|request: &PropertyRequest| self.resolve(request));
        }
        if request.property_set() == Some(MEASURED_SET) {
            return self.measures.resolve(request);
        }
        if request.property_set() == Some(axioval_ir::SOURCE_SET) {
            return self.sources.resolve(request);
        }
        if request.property_set() == Some(axioval_ir::MEMBER_SET) {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        match &self.inner {
            Some(inner) => inner.resolve(request),
            None => Err(PropertyResolutionError::Unavailable(
                "property-resolution service is not registered".into(),
            )),
        }
    }

    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        match &self.inner {
            Some(inner) => inner.enumerate(request),
            None => Err(PropertyResolutionError::Unavailable(
                "property-resolution service is not registered".into(),
            )),
        }
    }
}

/// Installs the run's resolver over the host's in `services`: derived
/// sets answered from `classifications`, everything else by the host.
pub(crate) fn install(
    services: &mut crate::ServiceRegistry,
    host: Option<&PropertyResolutionServiceHandle>,
    classifications: Arc<Classifications>,
    groups: Arc<DerivedGroups>,
    project: &axioval_ir::Project,
) {
    use crate::SnapshotBoundService as _;
    let snapshots = host.map_or_else(Vec::new, |host| host.source_snapshots().to_vec());
    // A fresh cache per install: values read the classes and groups
    // derived so far, and are computed again once more are derived.
    let values = Arc::new(DerivedValues::new(
        services
            .get::<ValueExpressions>()
            .map(|expressions| expressions.0.clone())
            .unwrap_or_default(),
        services.get::<crate::ConceptBindings>().cloned(),
        services
            .get::<Arc<crate::expression::EvaluationBudget>>()
            .cloned(),
        services
            .get::<Arc<crate::expression::DeclaredTypes>>()
            .cloned(),
    ));
    services.replace(PropertyResolutionServiceHandle::new(Arc::new(
        DerivedProperties {
            inner: host.cloned(),
            measures: Measures::of(services, host, project),
            sources: SourceFacts::of(services),
            classifications: classifications.clone(),
            groups,
            values: values.clone(),
            snapshots,
        },
    )));
    services.replace(classifications);
    services.replace(values);
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::contract::{ClassificationRow, LocalizedText, Selector};

    fn definition(mode: ClassificationMode) -> ClassificationDefinition {
        let row = |class: &str| ClassificationRow {
            selector: Selector::All,
            class: class.into(),
        };
        ClassificationDefinition {
            id: "use".into(),
            name: LocalizedText::plain("use"),
            description: None,
            mode,
            rows: vec![row("office"), row("lab"), row("office")],
            classes: Vec::new(),
        }
    }

    fn verdicts(verdicts: &[Option<bool>]) -> impl Iterator<Item = SelectorVerdict> + '_ {
        verdicts.iter().map(|verdict| match verdict {
            Some(true) => SelectorVerdict::Match(Vec::new()),
            Some(false) => SelectorVerdict::NoMatch(Vec::new()),
            None => SelectorVerdict::Undecided(NotEvaluatedReason::InvalidEvidence, "?".into()),
        })
    }

    #[test]
    fn the_first_decided_match_wins_and_an_undecided_row_before_it_decides_nothing() {
        let first = definition(ClassificationMode::FirstMatch);
        assert_eq!(
            classify_one(&first, verdicts(&[Some(false), Some(true), None])),
            ClassOutcome::Classified {
                classes: vec!["lab".into()],
                rows: vec![1]
            }
        );
        assert!(matches!(
            classify_one(&first, verdicts(&[None, Some(true), Some(true)])),
            ClassOutcome::Undecided(..)
        ));
        assert_eq!(
            classify_one(&first, verdicts(&[Some(false); 3])),
            ClassOutcome::Unclassified
        );
    }

    #[test]
    fn all_match_names_every_matching_class_once_and_needs_every_row() {
        let all = definition(ClassificationMode::AllMatch);
        assert_eq!(
            classify_one(&all, verdicts(&[Some(true), Some(true), Some(true)])),
            ClassOutcome::Classified {
                classes: vec!["office".into(), "lab".into()],
                rows: vec![0, 1, 2]
            }
        );
        assert!(matches!(
            classify_one(&all, verdicts(&[Some(true), Some(false), None])),
            ClassOutcome::Undecided(..)
        ));
    }

    /// A three-level tree with `w1` in the leaf `331`, `w2` in the inner
    /// class `340`, `w3` in none, and `w4` undecided.
    fn cost_groups(mode: ClassificationMode) -> (Classifications, Vec<ObjectId>) {
        use axioval_ir::SourceId;
        use axioval_ir::contract::ClassDefinition;
        let class = |id: &str, parent: Option<&str>| ClassDefinition {
            id: id.into(),
            code: Some(id.into()),
            name: LocalizedText::plain(id),
            parent: parent.map(Into::into),
        };
        let mut definition = definition(mode);
        definition.classes = vec![
            class("300", None),
            class("330", Some("300")),
            class("331", Some("330")),
            class("340", Some("300")),
        ];
        let tree = ClassTree::of(&ClassificationDefinition {
            rows: vec![ClassificationRow {
                selector: Selector::All,
                class: "331".into(),
            }],
            ..definition
        })
        .unwrap();
        let source = SourceId::new("t", "model").unwrap();
        let objects: Vec<ObjectId> = ["w1", "w2", "w3", "w4"]
            .iter()
            .map(|local| ObjectId::new(source.clone(), *local).unwrap())
            .collect();
        let classified = |classes: &[&str]| ClassOutcome::Classified {
            classes: classes.iter().map(|class| (*class).to_owned()).collect(),
            rows: vec![0],
        };
        let outcomes = [
            classified(&["331"]),
            classified(&["340"]),
            ClassOutcome::Unclassified,
            ClassOutcome::Undecided(NotEvaluatedReason::InvalidEvidence, "?".into()),
        ];
        let classifications = Classifications {
            modes: BTreeMap::from([("use".to_owned(), mode)]),
            trees: BTreeMap::from([("use".to_owned(), tree)]),
            outcomes: BTreeMap::from([(
                "use".to_owned(),
                objects.iter().cloned().zip(outcomes).collect(),
            )]),
        };
        (classifications, objects)
    }

    fn read(
        classifications: &Classifications,
        object: &ObjectId,
        name: &str,
    ) -> Result<Option<PropertyValue>, PropertyResolutionError> {
        let request =
            PropertyRequest::try_new(object.clone(), Some(CLASSIFICATION_SET.into()), name)?;
        Ok(match classifications.resolve(&request)? {
            PropertyResolution::Present(property) => Some(property.property().value.clone()),
            PropertyResolution::Absent(_) => None,
        })
    }

    #[test]
    fn a_level_reads_the_class_there_on_the_way_to_the_root() {
        let (classifications, objects) = cost_groups(ClassificationMode::FirstMatch);
        let text = |value: &str| Some(PropertyValue::String(value.into()));
        assert_eq!(read(&classifications, &objects[0], "use"), Ok(text("331")));
        assert_eq!(
            read(&classifications, &objects[0], "use;level=1"),
            Ok(text("300"))
        );
        assert_eq!(
            read(&classifications, &objects[0], "use;level=2"),
            Ok(text("330"))
        );
        assert_eq!(
            read(&classifications, &objects[1], "use;level=2"),
            Ok(text("340"))
        );
        // An inner class has no class below its level: an exact absence.
        assert_eq!(read(&classifications, &objects[1], "use;level=3"), Ok(None));
        assert_eq!(read(&classifications, &objects[2], "use;level=1"), Ok(None));
        assert!(matches!(
            read(&classifications, &objects[3], "use;level=1"),
            Err(PropertyResolutionError::Incomplete(_))
        ));
    }

    #[test]
    fn a_class_selects_itself_and_with_descendants_the_classes_below() {
        let (classifications, objects) = cost_groups(ClassificationMode::AllMatch);
        let selects = |class: &str, descendants: bool, object: &ObjectId| {
            classifications.selects("use", class, descendants, object)
        };
        assert_eq!(selects("331", false, &objects[0]), Ok(true));
        assert_eq!(selects("330", false, &objects[0]), Ok(false));
        assert_eq!(selects("330", true, &objects[0]), Ok(true));
        assert_eq!(selects("300", true, &objects[1]), Ok(true));
        assert_eq!(selects("330", true, &objects[1]), Ok(false));
        assert_eq!(selects("300", true, &objects[2]), Ok(false));
        assert!(selects("300", true, &objects[3]).is_err());
        assert!(
            classifications
                .selects("other", "300", true, &objects[0])
                .is_err()
        );
    }
}
