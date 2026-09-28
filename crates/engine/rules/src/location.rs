//! Locating outcomes by storey and space, as the host's policy says.
//!
//! A location is a reading aid, never a verdict: it decides nothing about
//! the finding. Still, a location that could not be derived is said so
//! (`Location::unresolved`) rather than left looking complete, so a reader
//! filtering by storey never loses an outcome that might be there.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    LocationMethod, LocationPolicy, PropertyRequest, PropertyResolution,
    PropertyResolutionServiceHandle, RuleContext,
};
use axioval_ir::{Location, Object, ObjectId, Place, PropertyValue};

use crate::support::{Traversal, Unavailable};

/// The geometry-derived relationship from an element to the spaces whose
/// body contains or meets it.
const CONTAINED_IN_SPACE: &str = "axioval:derived.contained-in-space:forward";

/// Where one object lies.
#[derive(Clone, Default)]
struct Located {
    storeys: BTreeSet<ObjectId>,
    spaces: BTreeSet<ObjectId>,
    doubt: Option<String>,
}

impl Located {
    fn doubt(&mut self, why: String) {
        self.doubt.get_or_insert(why);
    }
}

/// Locates the objects of one rule's outcomes, each object once.
pub(crate) struct Locator<'p> {
    policy: &'p LocationPolicy,
    storeys: BTreeSet<ObjectId>,
    spaces: BTreeSet<ObjectId>,
    located: BTreeMap<ObjectId, Located>,
    names: BTreeMap<ObjectId, Option<String>>,
}

impl<'p> Locator<'p> {
    pub(crate) fn new(context: &RuleContext<'_>, policy: &'p LocationPolicy) -> Self {
        let of = |kinds: &[String]| -> BTreeSet<ObjectId> {
            context
                .project
                .objects()
                .filter(|object| {
                    kinds
                        .iter()
                        .any(|kind| object.kind().eq_ignore_ascii_case(kind))
                })
                .map(|object| object.id.clone())
                .collect()
        };
        Self {
            policy,
            storeys: of(&policy.storey_kinds),
            spaces: of(&policy.space_kinds),
            located: BTreeMap::new(),
            names: BTreeMap::new(),
        }
    }

    /// The storeys and spaces `objects` lie in together.
    pub(crate) fn location<'a>(
        &mut self,
        context: &RuleContext<'_>,
        objects: impl IntoIterator<Item = &'a ObjectId>,
    ) -> Location {
        let mut together = Located::default();
        for id in objects {
            let located = self.locate(context, id);
            together.storeys.extend(located.storeys.iter().cloned());
            together.spaces.extend(located.spaces.iter().cloned());
            if let Some(why) = &located.doubt {
                together.doubt(why.clone());
            }
        }
        let storeys = together
            .storeys
            .iter()
            .map(|id| self.place(context, id))
            .collect();
        let spaces = together
            .spaces
            .iter()
            .map(|id| self.place(context, id))
            .collect();
        Location {
            storeys,
            spaces,
            unresolved: together.doubt,
        }
    }

    fn locate(&mut self, context: &RuleContext<'_>, id: &ObjectId) -> Located {
        if let Some(located) = self.located.get(id) {
            return located.clone();
        }
        let located = match context.project.object(id) {
            Some(object) => self.derive(context, object),
            // A resource object is placed nowhere: it has no location, and
            // nothing about it is in doubt.
            None if crate::selection::is_resource(context, id) => Located::default(),
            None => Located {
                doubt: Some(format!("{id} is not in the project")),
                ..Located::default()
            },
        };
        self.located.insert(id.clone(), located.clone());
        located
    }

    fn derive(&mut self, context: &RuleContext<'_>, object: &Object) -> Located {
        let mut located = Located::default();
        let id = &object.id;
        if self.storeys.contains(id) {
            located.storeys.insert(id.clone());
        } else {
            match self.climb(context, id, &self.storeys) {
                Ok(found) => located.storeys = found,
                Err((_, why)) => located.doubt(format!("the storey of {id}: {why}")),
            }
        }
        if self.spaces.contains(id) {
            located.spaces.insert(id.clone());
        } else {
            let spaces = match self.policy.method {
                LocationMethod::Storeys => Ok(BTreeSet::new()),
                LocationMethod::Containers => self.climb(context, id, &self.spaces),
                LocationMethod::Geometry => self.derived_spaces(context, id),
            };
            match spaces {
                Ok(found) => located.spaces = found,
                Err((_, why)) => located.doubt(format!("the space of {id}: {why}")),
            }
        }
        // An element placed in no storey itself lies on its spaces' storeys.
        if located.storeys.is_empty() && !self.storeys.contains(id) {
            let spaces: Vec<ObjectId> = located
                .spaces
                .iter()
                .filter(|space| *space != id)
                .cloned()
                .collect();
            for space in spaces {
                let of_space = self.locate(context, &space);
                located.storeys.extend(of_space.storeys);
                if let Some(why) = of_space.doubt {
                    located.doubt(why);
                }
            }
        }
        located
    }

    /// The nearest `containers` above `id` along the containment path.
    fn climb(
        &self,
        context: &RuleContext<'_>,
        id: &ObjectId,
        containers: &BTreeSet<ObjectId>,
    ) -> Result<BTreeSet<ObjectId>, Unavailable> {
        if self.policy.containment.is_empty() || containers.is_empty() {
            return Ok(BTreeSet::new());
        }
        let climb = Traversal::path(&self.policy.containment)?;
        Ok(climb.nearest_containers(context, id, containers)?.0)
    }

    /// The spaces whose body contains or meets `id`.
    fn derived_spaces(
        &self,
        context: &RuleContext<'_>,
        id: &ObjectId,
    ) -> Result<BTreeSet<ObjectId>, Unavailable> {
        if self.spaces.is_empty() {
            return Ok(BTreeSet::new());
        }
        let step = [CONTAINED_IN_SPACE.to_owned()];
        let universe: Vec<&Object> = self
            .spaces
            .iter()
            .filter_map(|space| context.project.object(space))
            .collect();
        let (reached, _) = Traversal::path(&step)?.related(context, id, &universe)?;
        Ok(reached.into_iter().collect())
    }

    /// A storey or space with the name the source gives it, read natively.
    /// A name that cannot be read is left out: it only labels the place.
    fn place(&mut self, context: &RuleContext<'_>, id: &ObjectId) -> Place {
        let name = if let Some(name) = self.names.get(id) {
            name.clone()
        } else {
            let name = self.name(context, id);
            self.names.insert(id.clone(), name.clone());
            name
        };
        Place {
            id: id.clone(),
            name,
        }
    }

    fn name(&self, context: &RuleContext<'_>, id: &ObjectId) -> Option<String> {
        let (set, property) = self.policy.name.as_ref()?;
        let service = context.services.get::<PropertyResolutionServiceHandle>()?;
        let request = PropertyRequest::try_new(id.clone(), Some(set.clone()), property).ok()?;
        match service.resolve(&request).ok()? {
            PropertyResolution::Present(resolved) => match resolved.property().value() {
                PropertyValue::String(text) if !text.trim().is_empty() => {
                    Some(text.trim().to_owned())
                }
                _ => None,
            },
            PropertyResolution::Absent(_) => None,
        }
    }
}
