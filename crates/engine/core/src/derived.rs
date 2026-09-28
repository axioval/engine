//! Properties the engine derives instead of reading them from a source: the
//! class names a ruleset's classifications assign
//! ([`axioval_ir::CLASSIFICATION_SET`]).
//!
//! The runtime answers them through the one property-resolution handle, so
//! every selector and capability reads a derived property exactly as it
//! reads a stated one. What cannot be derived is an error of the request,
//! never an absence.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_ir::contract::{ClassificationDefinition, ClassificationMode};
use axioval_ir::{
    CLASSIFICATION_SET, Evidence, NotEvaluatedReason, ObjectId, Property, PropertyValue,
};

use crate::properties::{
    CompletePropertyAbsenceEvidence, PropertyEnumeration, PropertyEnumerationRequest,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty,
};
use crate::session::SourceSnapshot;
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

    /// Classifies every object of the context's project by `definition`,
    /// its rows evaluated by `refiner`, and adds the result.
    pub(crate) fn classify(
        &mut self,
        refiner: &dyn OutcomeRefiner,
        context: &RuleContext<'_>,
        definition: &ClassificationDefinition,
    ) {
        let outcomes = context
            .project
            .objects()
            .map(|object| {
                let rows = definition
                    .rows
                    .iter()
                    .map(|row| refiner.evaluate_selector(context, &row.selector, object));
                (object.id.clone(), classify_one(definition, rows))
            })
            .collect();
        self.modes.insert(definition.id.clone(), definition.mode);
        self.outcomes.insert(definition.id.clone(), outcomes);
    }

    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let id = request.property();
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
                let value = match self.modes[id] {
                    ClassificationMode::FirstMatch => PropertyValue::String(classes[0].clone()),
                    ClassificationMode::AllMatch => PropertyValue::List(
                        classes.iter().cloned().map(PropertyValue::String).collect(),
                    ),
                };
                let rows = rows
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let property = Property::new(CLASSIFICATION_SET, id, value)
                    .map_err(|_| PropertyResolutionError::InvalidRequest)?
                    .with_evidence(evidence(format!("{CLASSIFICATION_SET}/{id}#rows={rows}")));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            ClassOutcome::Unclassified => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    evidence(format!("{CLASSIFICATION_SET}/{id}#no-row")),
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

/// The run's property resolver: derived sets answered by the engine, every
/// other request by the host's resolver.
pub(crate) struct DerivedProperties {
    pub(crate) inner: Option<PropertyResolutionServiceHandle>,
    pub(crate) classifications: Arc<Classifications>,
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
        if request.property_set() == Some(CLASSIFICATION_SET) {
            return self.classifications.resolve(request);
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
) {
    use crate::SnapshotBoundService as _;
    let snapshots = host.map_or_else(Vec::new, |host| host.source_snapshots().to_vec());
    services.replace(PropertyResolutionServiceHandle::new(Arc::new(
        DerivedProperties {
            inner: host.cloned(),
            classifications: classifications.clone(),
            snapshots,
        },
    )));
    services.replace(classifications);
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
}
