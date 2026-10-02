//! Groups a ruleset derives from its members (`groupings`).
//!
//! A grouping selects its members and groups them by a key: equal values of
//! a property, or equal codes in a classification system. Each group is a
//! derived object of kind [`DERIVED_GROUP_KIND`], never a model object: a
//! `derivedGroup` selector reaches it (through [`ResourceObjects`]), the
//! relationship `axioval:derived.group;by=<id>` runs from each member to its
//! group, the reserved set [`GROUP_SET`] states its key and member count,
//! and its footprint is the union of its members' footprints.
//!
//! Nothing undecided is guessed. A member without a key is ungrouped. A
//! member whose selection or key cannot be read could belong to any group of
//! its source, or to one not listed: every group of that source is then
//! undecided, and so is the list of the source's groups.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::contract::{GroupingDefinition, GroupingKey};
use axioval_ir::{
    DERIVED_GROUP_KIND, Evidence, GROUP_KEY, GROUP_MEMBERS, GROUP_SET, NotEvaluatedReason, Object,
    ObjectId, Project, Property, PropertyValue, SourceId,
};

use crate::classifications::ClassificationServiceHandle;
use crate::derived_relationships::DERIVED_RELATIONSHIP_PREFIX;
use crate::plan_area::{PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle};
use crate::properties::{
    CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution, PropertyResolutionError,
    ResolvedProperty,
};
use crate::relationships::{
    CompleteRelationshipEdges, CompleteRelationshipSelection, RelationshipEdgesRequest,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, TraversalDirection,
};
use crate::session::SourceSnapshot;
use crate::{OutcomeRefiner, ResourceObjects, RuleContext, SelectorVerdict, ServiceRegistry};

/// The relationship identity of a grouping's members and groups, before
/// the grouping's id: `axioval:derived.group;by=<id>`.
pub const GROUP_RELATIONSHIP_PREFIX: &str = "axioval:derived.group;by=";

/// What reading one property of one object gave, for the runtime's own
/// reads of the model ([`OutcomeRefiner::read_property`]).
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyRead {
    /// The value, with its evidence.
    Present(PropertyValue, Vec<Evidence>),
    /// Surely absent, with the proof.
    Absent(Vec<Evidence>),
    /// Cannot be read, and why.
    Undecided(NotEvaluatedReason, String),
}

/// Where one selected object stands in a grouping.
#[derive(Clone, Debug, PartialEq)]
pub enum Membership {
    /// A member of the group, with the facts that decided its key.
    Grouped(ObjectId, Vec<Evidence>),
    /// Selected, but without a key: in no group.
    Ungrouped(Vec<Evidence>),
    /// Its selection or its key cannot be read, and why.
    Undecided(NotEvaluatedReason, String),
}

/// One derived group.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    key: String,
    members: Vec<ObjectId>,
    undecided: Option<String>,
}

impl Group {
    /// The key its members share, as text.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Its members, in identity order.
    #[must_use]
    pub fn members(&self) -> &[ObjectId] {
        &self.members
    }

    /// Why its members are not known completely, if they are not.
    #[must_use]
    pub fn undecided(&self) -> Option<&str> {
        self.undecided.as_deref()
    }
}

/// Every group of one grouping, and where each selected object stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grouping {
    groups: BTreeMap<ObjectId, Group>,
    members: BTreeMap<ObjectId, Membership>,
    /// Sources whose list of groups is incomplete, and why.
    incomplete: BTreeMap<SourceId, String>,
}

impl Grouping {
    /// A grouping of `members`, each with its membership.
    ///
    /// A group is listed for every distinct group a `Grouped` member names;
    /// an `Undecided` member leaves every group of its source undecided and
    /// the source's list of groups incomplete.
    #[must_use]
    pub fn of(
        id: &str,
        members: impl IntoIterator<Item = (ObjectId, Membership)>,
        keys: &BTreeMap<ObjectId, String>,
    ) -> Self {
        let mut grouping = Self::default();
        for (member, membership) in members {
            match &membership {
                Membership::Grouped(group, _) => {
                    grouping
                        .groups
                        .entry(group.clone())
                        .or_insert_with(|| Group {
                            key: keys.get(group).cloned().unwrap_or_default(),
                            members: Vec::new(),
                            undecided: None,
                        })
                        .members
                        .push(member.clone());
                }
                Membership::Undecided(_, why) => {
                    grouping
                        .incomplete
                        .entry(member.source.clone())
                        .or_insert_with(|| {
                            format!("the membership of {member} in `{id}` is undecided: {why}")
                        });
                }
                Membership::Ungrouped(_) => {}
            }
            grouping.members.insert(member, membership);
        }
        for (group_id, group) in &mut grouping.groups {
            group.members.sort();
            group.undecided = grouping.incomplete.get(&group_id.source).cloned();
        }
        grouping
    }

    /// Marks `group` undecided, for a reason only its derivation knows.
    pub fn undecide(&mut self, group: &ObjectId, why: impl Into<String>) {
        if let Some(group) = self.groups.get_mut(group) {
            group.undecided.get_or_insert(why.into());
        }
    }

    /// Every group, by identity.
    #[must_use]
    pub fn groups(&self) -> &BTreeMap<ObjectId, Group> {
        &self.groups
    }

    /// The group `id`, if it is one of this grouping's.
    #[must_use]
    pub fn group(&self, id: &ObjectId) -> Option<&Group> {
        self.groups.get(id)
    }

    /// Where `object` stands; `None` when the grouping does not select it.
    #[must_use]
    pub fn membership(&self, object: &ObjectId) -> Option<&Membership> {
        self.members.get(object)
    }

    /// Sources whose list of groups is incomplete, and why.
    #[must_use]
    pub fn incomplete(&self) -> &BTreeMap<SourceId, String> {
        &self.incomplete
    }
}

/// Every grouping a run derives, by id.
#[derive(Clone, Debug, Default)]
pub struct DerivedGroups {
    groupings: BTreeMap<String, Grouping>,
}

impl DerivedGroups {
    /// Adds the grouping `id`.
    #[must_use]
    pub fn with_grouping(mut self, id: impl Into<String>, grouping: Grouping) -> Self {
        self.groupings.insert(id.into(), grouping);
        self
    }

    /// The grouping `id`, if the run derives it.
    #[must_use]
    pub fn grouping(&self, id: &str) -> Option<&Grouping> {
        self.groupings.get(id)
    }

    /// The grouping and group `id` is, if it is a derived group.
    #[must_use]
    pub fn group(&self, id: &ObjectId) -> Option<(&str, &Group)> {
        self.groupings
            .iter()
            .find_map(|(name, grouping)| Some((name.as_str(), grouping.group(id)?)))
    }

    /// Answers a request in [`GROUP_SET`], or any request about a derived
    /// group outside [`axioval_ir::MEASURED_SET`]: a group states its key
    /// and member count and nothing else, and no other object states either.
    pub(crate) fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let object = request.object_id();
        let name = request.property();
        let absent = |why: &str| {
            Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(object.source.clone(), format!("{GROUP_SET}/{name}#{why}")),
                )?,
            ))
        };
        let Some((grouping, group)) = self.group(object) else {
            return absent("not-a-derived-group");
        };
        if request.property_set() != Some(GROUP_SET) {
            return absent("derived-group");
        }
        let value = match name {
            GROUP_KEY => PropertyValue::String(group.key.clone()),
            GROUP_MEMBERS => {
                if let Some(why) = &group.undecided {
                    return Err(PropertyResolutionError::Incomplete(why.clone()));
                }
                PropertyValue::Integer(i64::try_from(group.members.len()).unwrap_or(i64::MAX))
            }
            _ => return Err(PropertyResolutionError::InvalidRequest),
        };
        let property = Property::new(GROUP_SET, name, value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?
            .with_evidence(Evidence::exact(
                object.source.clone(),
                format!("{GROUP_RELATIONSHIP_PREFIX}{grouping}:{object}#{name}"),
            ));
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
    }
}

/// The identity of the group of `grouping` keyed `key` in `source`.
///
/// # Errors
///
/// A blank key, which no group has.
pub fn group_id(source: &SourceId, grouping: &str, key: &str) -> Result<ObjectId, String> {
    ObjectId::new(
        source.clone(),
        format!("{DERIVED_GROUP_KIND}/{grouping}/{key}"),
    )
    .map_err(|error| error.to_string())
}

/// Derives the grouping `definition` over every object of the context's
/// project: its members by `refiner`, their keys as the definition says.
pub(crate) fn derive(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    definition: &GroupingDefinition,
) -> Grouping {
    let mut keys = BTreeMap::new();
    let mut members = Vec::new();
    for object in context.project.objects() {
        let membership = match refiner.evaluate_selector(context, &definition.members, object) {
            SelectorVerdict::NoMatch(_) => continue,
            SelectorVerdict::Undecided(reason, why) => Membership::Undecided(reason, why),
            SelectorVerdict::Match(_) => match key(refiner, context, &definition.by, object) {
                Ok(Some((key, evidence))) => {
                    match group_id(&object.id.source, &definition.id, &key) {
                        Ok(group) => {
                            keys.insert(group.clone(), key);
                            Membership::Grouped(group, evidence)
                        }
                        Err(why) => Membership::Undecided(NotEvaluatedReason::InvalidEvidence, why),
                    }
                }
                Ok(None) => Membership::Ungrouped(Vec::new()),
                Err((reason, why)) => Membership::Undecided(reason, why),
            },
        };
        members.push((object.id.clone(), membership));
    }
    Grouping::of(&definition.id, members, &keys)
}

type Key = Result<Option<(String, Vec<Evidence>)>, (NotEvaluatedReason, String)>;

/// A member's key: `None` when it has none.
fn key(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    by: &GroupingKey,
    object: &Object,
) -> Key {
    match by {
        GroupingKey::Property {
            property_set,
            property,
        } => match refiner.read_property(context, object, property_set.as_deref(), property) {
            PropertyRead::Present(value, evidence) => {
                let text = match value {
                    PropertyValue::Null => return Ok(None),
                    PropertyValue::String(text) if text.trim().is_empty() => return Ok(None),
                    PropertyValue::String(text) => text,
                    PropertyValue::Integer(value) => value.to_string(),
                    PropertyValue::Boolean(value) => value.to_string(),
                    PropertyValue::Decimal(value) if value.is_finite() => value.to_string(),
                    other => {
                        return Err((
                            NotEvaluatedReason::InvalidEvidence,
                            format!(
                                "{} states {property} as {other:?}, not one text, integer, \
                                 boolean or number to group by",
                                object.id
                            ),
                        ));
                    }
                };
                Ok(Some((text, evidence)))
            }
            PropertyRead::Absent(_) => Ok(None),
            PropertyRead::Undecided(reason, why) => Err((reason, why)),
        },
        GroupingKey::Classification { system } => {
            let Some(service) = context.services.get::<ClassificationServiceHandle>() else {
                return Err((
                    NotEvaluatedReason::MissingService,
                    "no classification service is registered".into(),
                ));
            };
            let assignments = service
                .classifications(&object.id)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
            let mut codes = BTreeSet::new();
            for assignment in &assignments {
                match assignment.in_system(system) {
                    None => {
                        return Err((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!(
                                "a classification of {} names no system, so whether it is in \
                                 `{system}` is unknown",
                                object.id
                            ),
                        ));
                    }
                    Some(false) => {}
                    Some(true) => match assignment.codes.first().cloned().flatten() {
                        Some(code) => {
                            codes.insert(code);
                        }
                        None => {
                            return Err((
                                NotEvaluatedReason::IncompleteEvidence,
                                format!("{} is classified in `{system}` without a code", object.id),
                            ));
                        }
                    },
                }
            }
            let mut codes = codes.into_iter();
            match (codes.next(), codes.next()) {
                (None, _) => Ok(None),
                (Some(code), None) => Ok(Some((
                    code.clone(),
                    vec![Evidence::exact(
                        object.id.source.clone(),
                        format!("classification:{system}:{code}:{}", object.id),
                    )],
                ))),
                (Some(first), Some(second)) => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} carries several codes in `{system}` ({first}, {second}, …), so its \
                         group is undecided",
                        object.id
                    ),
                )),
            }
        }
    }
}

/// The derived group objects of `grouping`.
pub(crate) fn objects(grouping: &Grouping) -> Vec<Object> {
    grouping
        .groups
        .keys()
        .map(|id| Object::new(id.clone(), DERIVED_GROUP_KIND))
        .collect()
}

/// Installs the run's derived groups: their objects in the population,
/// their relationship beside the host's, and their footprints beside the
/// host's plan areas.
pub(crate) fn install(services: &mut ServiceRegistry, groups: &Arc<DerivedGroups>) {
    let mut population = services
        .get::<ResourceObjects>()
        .cloned()
        .unwrap_or_default();
    for (id, grouping) in &groups.groupings {
        population = population.with_groups(
            id.clone(),
            objects(grouping),
            grouping
                .incomplete
                .iter()
                .map(|(source, why)| (source.clone(), why.clone()))
                .collect(),
        );
    }
    services.replace(population);
    let inner = services
        .get::<RelationshipSelectionServiceHandle>()
        .cloned();
    services.replace(RelationshipSelectionServiceHandle::new(Arc::new(
        GroupRelationships {
            inner,
            groups: groups.clone(),
        },
    )));
    if let Some(inner) = services.get::<PlanAreaServiceHandle>().cloned() {
        services.replace(PlanAreaServiceHandle::new(Arc::new(GroupAreas {
            inner,
            groups: groups.clone(),
        })));
    }
    services.replace(groups.clone());
}

/// Answers `axioval:derived.group;by=<id>` from the run's groups, every
/// other identity from the host's relationship service.
struct GroupRelationships {
    inner: Option<RelationshipSelectionServiceHandle>,
    groups: Arc<DerivedGroups>,
}

impl GroupRelationships {
    fn answer(
        &self,
        id: &str,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let grouping = self
            .groups
            .grouping(id)
            .ok_or(RelationshipSelectionError::InvalidRequest)?;
        let identity = format!("{GROUP_RELATIONSHIP_PREFIX}{id}");
        let anchor = request.anchor();
        let unavailable = |why: &str| RelationshipSelectionError::Unavailable(why.to_owned());
        let mut evidence = vec![Evidence::exact(
            anchor.source.clone(),
            format!("{identity}:derived-from:{anchor}"),
        )];
        let mut reached = BTreeSet::new();
        // The group the anchor is a member of.
        let up = |evidence: &mut Vec<Evidence>| -> Result<Option<ObjectId>, _> {
            match grouping.membership(anchor) {
                None | Some(Membership::Ungrouped(_)) => Ok(None),
                Some(Membership::Undecided(_, why)) => Err(unavailable(why)),
                Some(Membership::Grouped(group, cited)) => {
                    evidence.extend(cited.iter().cloned());
                    evidence.push(Evidence::exact(
                        anchor.source.clone(),
                        format!("{identity}:{anchor}->{group}"),
                    ));
                    Ok(Some(group.clone()))
                }
            }
        };
        // The members of a group.
        let down = |group: &ObjectId, evidence: &mut Vec<Evidence>| -> Result<Vec<ObjectId>, _> {
            let Some(found) = grouping.group(group) else {
                return Ok(Vec::new());
            };
            if let Some(why) = &found.undecided {
                return Err(unavailable(why));
            }
            evidence.push(Evidence::exact(
                group.source.clone(),
                format!("{identity}:{group}:members={}", found.members.len()),
            ));
            Ok(found.members.clone())
        };
        match request.query() {
            RelationshipQuery::SharedGroup { .. } => {
                if let Some(group) = up(&mut evidence)? {
                    reached.extend(down(&group, &mut evidence)?);
                }
            }
            RelationshipQuery::Related { direction, .. } => {
                if matches!(
                    direction,
                    TraversalDirection::Forward | TraversalDirection::Either
                ) {
                    reached.extend(up(&mut evidence)?);
                }
                if matches!(
                    direction,
                    TraversalDirection::Backward | TraversalDirection::Either
                ) {
                    reached.extend(down(anchor, &mut evidence)?);
                }
            }
        }
        reached.remove(anchor);
        let candidates = request
            .candidate_universe()
            .iter()
            .filter(|candidate| reached.contains(*candidate))
            .cloned()
            .collect();
        let mut unique = BTreeSet::new();
        evidence.retain(|item| unique.insert((item.source.clone(), item.locator.clone())));
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

impl RelationshipSelectionService for GroupRelationships {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        use crate::SnapshotBoundService as _;
        self.inner
            .as_ref()
            .map_or(&[], |inner| inner.source_snapshots())
    }

    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        if let Some(id) = request
            .query()
            .relationship()
            .as_str()
            .strip_prefix(GROUP_RELATIONSHIP_PREFIX)
        {
            return self.answer(id, request);
        }
        match &self.inner {
            Some(inner) => inner.select(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }

    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        if request
            .relationship()
            .as_str()
            .starts_with(DERIVED_RELATIONSHIP_PREFIX)
        {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "derived relationships are answered per object; the edges of `{}` are not listed",
                request.relationship().as_str()
            )));
        }
        match &self.inner {
            Some(inner) => inner.edges(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }
}

/// A derived group's footprint, the union of its members' footprints,
/// beside the host's plan areas.
struct GroupAreas {
    inner: PlanAreaServiceHandle,
    groups: Arc<DerivedGroups>,
}

impl GroupAreas {
    fn refuse_groups(&self, objects: &[&ObjectId]) -> Result<(), PlanAreaError> {
        match objects
            .iter()
            .find(|object| self.groups.group(object).is_some())
        {
            Some(group) => Err(PlanAreaError::Unavailable(format!(
                "{group} is a derived group, measured by its footprint only"
            ))),
            None => Ok(()),
        }
    }
}

impl PlanAreaService for GroupAreas {
    /// Each member's footprint outside the members before it, summed: the
    /// union, every overlap counted once.
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let Some((grouping, group)) = self.groups.group(object) else {
            return self.inner.measure_footprint(object);
        };
        if let Some(why) = &group.undecided {
            return Err(PlanAreaError::Unavailable(why.clone()));
        }
        let (mut lower, mut upper) = (0.0, 0.0);
        for (index, member) in group.members.iter().enumerate() {
            let piece = self
                .inner
                .measure_uncovered_area(member, &group.members[..index], 0.0)?;
            lower += piece.lower_square_metres();
            upper += piece.upper_square_metres();
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        let locator = format!(
            "{GROUP_RELATIONSHIP_PREFIX}{grouping}:{object}:area-union-of={}",
            group.members.len()
        );
        let evidence = Evidence {
            source: object.source.clone(),
            locator,
            exact,
        };
        PlanArea::try_new(lower, upper, evidence)
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        self.refuse_groups(&[first, second])?;
        self.inner.measure_plan_overlap(first, second)
    }

    fn measure_uncovered_area(
        &self,
        object: &ObjectId,
        cover: &[ObjectId],
        growth_metres: f64,
    ) -> Result<PlanArea, PlanAreaError> {
        let mut all: Vec<&ObjectId> = cover.iter().collect();
        all.push(object);
        self.refuse_groups(&all)?;
        self.inner
            .measure_uncovered_area(object, cover, growth_metres)
    }

    fn measure_outside_bands(
        &self,
        object: &ObjectId,
        bands: &[crate::plan_area::PlanBand],
    ) -> Result<PlanArea, PlanAreaError> {
        self.refuse_groups(&[object])?;
        self.inner.measure_outside_bands(object, bands)
    }

    fn measure_coverage(
        &self,
        request: &crate::CoverageRequest,
    ) -> Result<crate::CoverageEvidence, PlanAreaError> {
        self.refuse_groups(&[request.subject()])?;
        self.inner.measure_coverage(request)
    }

    fn measure_elevation_cover(
        &self,
        request: &crate::plan_area::ElevationRequest,
    ) -> Result<crate::plan_area::ElevationCover, PlanAreaError> {
        self.refuse_groups(&[request.object()])?;
        self.inner.measure_elevation_cover(request)
    }
}

/// Derives every grouping of a run, in order, over `project`.
pub(crate) fn derive_all(
    refiner: &dyn OutcomeRefiner,
    project: &Project,
    services: &ServiceRegistry,
    definitions: &[GroupingDefinition],
) -> DerivedGroups {
    let context = RuleContext { project, services };
    definitions
        .iter()
        .fold(DerivedGroups::default(), |groups, definition| {
            groups.with_grouping(definition.id.clone(), derive(refiner, &context, definition))
        })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn source() -> SourceId {
        SourceId::new("t", "model").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    /// Rooms `a` (16 m²) and `b` (12 m²) overlap by 4 m²; `c` stands apart.
    struct Rooms;

    impl PlanAreaService for Rooms {
        fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
            self.measure_uncovered_area(object, &[], 0.0)
        }
        fn measure_plan_overlap(
            &self,
            _: &ObjectId,
            _: &ObjectId,
        ) -> Result<PlanArea, PlanAreaError> {
            Err(PlanAreaError::Unavailable("unused".into()))
        }
        fn measure_uncovered_area(
            &self,
            object: &ObjectId,
            cover: &[ObjectId],
            _: f64,
        ) -> Result<PlanArea, PlanAreaError> {
            let area = match (object.local_id.as_str(), cover.is_empty()) {
                ("a", true) => 16.0,
                ("b", true) => 12.0,
                ("b", false) => 8.0,
                _ => return Err(PlanAreaError::Unavailable("not measured".into())),
            };
            PlanArea::try_new(area, area, Evidence::exact(source(), "area"))
        }
    }

    fn grouping(undecided: bool) -> Arc<DerivedGroups> {
        let flat = id("axioval:group/flats/1");
        let mut members = vec![
            (id("a"), Membership::Grouped(flat.clone(), Vec::new())),
            (id("b"), Membership::Grouped(flat.clone(), Vec::new())),
            (id("c"), Membership::Ungrouped(Vec::new())),
        ];
        if undecided {
            members.push((
                id("d"),
                Membership::Undecided(NotEvaluatedReason::IncompleteEvidence, "unread".into()),
            ));
        }
        let keys = BTreeMap::from([(flat, "1".to_owned())]);
        Arc::new(
            DerivedGroups::default().with_grouping("flats", Grouping::of("flats", members, &keys)),
        )
    }

    #[test]
    fn a_groups_footprint_is_the_union_of_its_members() {
        let areas = GroupAreas {
            inner: PlanAreaServiceHandle::new(Arc::new(Rooms)),
            groups: grouping(false),
        };
        let area = areas
            .measure_footprint(&id("axioval:group/flats/1"))
            .unwrap();
        assert_eq!(
            (area.lower_square_metres(), area.upper_square_metres()),
            (24.0, 24.0)
        );
        assert!(area.is_exact());
        // A member stays measured by the host.
        assert_eq!(
            areas
                .measure_footprint(&id("a"))
                .unwrap()
                .upper_square_metres(),
            16.0
        );
        assert!(
            areas
                .measure_plan_overlap(&id("axioval:group/flats/1"), &id("a"))
                .is_err()
        );
        // An undecided member leaves the group's footprint unavailable.
        let undecided = GroupAreas {
            inner: PlanAreaServiceHandle::new(Arc::new(Rooms)),
            groups: grouping(true),
        };
        assert!(matches!(
            undecided.measure_footprint(&id("axioval:group/flats/1")),
            Err(PlanAreaError::Unavailable(_))
        ));
    }

    #[test]
    fn members_reach_their_group_and_a_group_its_members() {
        let relationships = GroupRelationships {
            inner: None,
            groups: grouping(false),
        };
        let request = |anchor: &str, direction| {
            RelationshipSelectionRequest::try_new(
                id(anchor),
                ["a", "b", "c", "axioval:group/flats/1"].map(id).to_vec(),
                RelationshipQuery::Related {
                    relationship: crate::SemanticRelationship::try_new(
                        "axioval:derived.group;by=flats",
                    )
                    .unwrap(),
                    direction,
                    follow_chain: false,
                },
            )
            .unwrap()
        };
        let reached = |anchor: &str, direction| -> Vec<String> {
            relationships
                .select(&request(anchor, direction))
                .unwrap()
                .candidates()
                .iter()
                .map(|object| object.local_id.clone())
                .collect()
        };
        assert_eq!(
            reached("a", TraversalDirection::Forward),
            ["axioval:group/flats/1"]
        );
        assert!(reached("c", TraversalDirection::Forward).is_empty());
        assert_eq!(
            reached("axioval:group/flats/1", TraversalDirection::Backward),
            ["a", "b"]
        );
        let undecided = GroupRelationships {
            inner: None,
            groups: grouping(true),
        };
        assert!(matches!(
            undecided.select(&request(
                "axioval:group/flats/1",
                TraversalDirection::Backward
            )),
            Err(RelationshipSelectionError::Unavailable(_))
        ));
    }
}
