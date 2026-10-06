//! Trusted built-in code measuring registered values the engine core does
//! not measure itself: values that reuse a capability's own measurement
//! (a distance in a mode, a clear width with its deductions, a coverage
//! share), so a measured value and the capability never disagree.
//!
//! A provider is registered beside the capabilities, in the
//! [`CapabilityRegistry`](crate::CapabilityRegistry), never by a package.
//! Each name it measures must be in the registry of measured values
//! ([`axioval_ir::measured`]) and measured by nothing else.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, Project, QuantityDimension, SourceId};

use crate::properties::PropertyResolutionError;
use crate::{RuleContext, ServiceRegistry};

/// What a provider measured.
///
/// A provider states the exactness of a value by the variant it answers;
/// none is ever inferred from a point, since a tessellation measures points
/// too. [`Measurement::Value`] is never exact, [`Measurement::Rounded`]
/// always, [`Measurement::Cited`] as its `exact` says. A provider measuring
/// from evidence answers exact only when every evidence it measured from is
/// exact (`crate::Evidence::exact`), and rounds any arithmetic of its own
/// outward.
#[derive(Clone, Debug, PartialEq)]
pub enum Measurement {
    /// A value sure to lie in `[lower, upper]` whose evidence is never
    /// exact, a point included: an approximation, or a value whose
    /// exactness the provider does not state. As a member's field it is
    /// exact as the member states ([`MeasuredMember::exact`]).
    Value {
        /// The least value it may have.
        lower: f64,
        /// The greatest value it may have.
        upper: f64,
        /// The value's dimension; `None` for a plain number, such as a
        /// count.
        dimension: Option<QuantityDimension>,
        /// Where the measurement came from, for its evidence.
        locator: String,
    },
    /// A value measured exactly, sure to lie in `[lower, upper]`: the
    /// interval holds only the rounding of exact arithmetic on exact
    /// positions, so its evidence is exact, as the measurement's own is.
    Rounded {
        /// The least value it may have.
        lower: f64,
        /// The greatest value it may have.
        upper: f64,
        /// The value's dimension; `None` for a plain number.
        dimension: Option<QuantityDimension>,
        /// Where the measurement came from, for its evidence.
        locator: String,
    },
    /// A value whose evidence is exact exactly when `exact` says so, as
    /// the capability measuring it cites its own: a count of what a search
    /// found, say, or a share whose interval holds an undecided cover
    /// measured on exact evidence. A member's field cited inexact is
    /// inexact whatever the member states.
    Cited {
        /// The least value it may have.
        lower: f64,
        /// The greatest value it may have.
        upper: f64,
        /// The value's dimension; `None` for a plain number.
        dimension: Option<QuantityDimension>,
        /// Where the measurement came from, for its evidence.
        locator: String,
        /// Whether the measurement is exact.
        exact: bool,
    },
    /// No value, known exactly: a path reaching nothing, a space with no
    /// obstacle above.
    Absent {
        /// Why there is none, for its evidence.
        locator: String,
    },
}

/// One field of a measured member.
#[derive(Clone, Debug, PartialEq)]
pub enum MemberValue {
    /// A number, or `null` ([`Measurement::Absent`]).
    Measured(Measurement),
    /// A truth, stated exactly.
    Truth {
        /// The truth.
        value: bool,
        /// Where it came from, for its evidence.
        locator: String,
    },
    /// A field the measurement could not decide; reading it leaves the
    /// expression not evaluated.
    Undecided {
        /// Why, in plain words.
        why: String,
    },
    /// Words naming what the member is, as messages name it (`the bottom
    /// of run 1 of 2`).
    Text {
        /// The words.
        text: String,
    },
    /// Objects the member was measured against or found (the obstacles
    /// governing a clearance, the doors standing on a landing), sorted by
    /// source-qualified identity; a finding on the member relates them.
    Objects {
        /// The objects.
        objects: Vec<ObjectId>,
    },
}

/// One measured member: a flight's step, a ramp's run.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredMember {
    /// Whether it surely is a member; `false` when the measurement could
    /// not decide it.
    pub certain: bool,
    /// Whether the measurement it comes from is exact: its fields' evidence
    /// is then exact, an interval holding only the rounding of exact
    /// arithmetic, unless a field is cited inexact itself. Stated by the
    /// provider from the evidence it measured from, never from its values.
    pub exact: bool,
    /// Every field its list declares, by name.
    pub fields: BTreeMap<&'static str, MemberValue>,
}

/// What a measurement was made against, beside its value: the objects a
/// finding on it relates (the doors a shelf length kept clear, the
/// counterparts a distance was measured to), sorted by source-qualified
/// identity, and the evidence that reached them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Citation {
    /// The objects measured against.
    pub related: Vec<ObjectId>,
    /// The evidence that reached them, beside the value's own.
    pub evidence: Vec<Evidence>,
    /// What the measurement found that its value does not say, each a
    /// clause a message may quote: why it may be larger or smaller than
    /// measured (a counterpart whose extent cannot be read), in the order
    /// found.
    pub notes: Vec<String>,
    /// The sources measured against, sorted by identity: the reference
    /// source a source is compared with. A message names them.
    pub sources: Vec<SourceId>,
}

/// Trusted code measuring registered values.
pub trait MeasuredProvider: Send + Sync + 'static {
    /// The registered names it measures.
    fn names(&self) -> &'static [&'static str];

    /// The registered member lists it measures.
    fn member_lists(&self) -> &'static [&'static str] {
        &[]
    }

    /// Measures the members `call` (one of [`Self::member_lists`]) lists
    /// of `object`, in the list's order.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let _ = (object, context);
        Err(PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}`",
            call.name()
        )))
    }

    /// [`Self::members`], with the evidence of the measurement the list
    /// comes from, cited even when it lists none; without it, none.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        self.members(call, object, context)
            .map(|members| (members, Vec::new()))
    }

    /// [`Self::measure`], with what the measurement was made against; by
    /// default nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        self.measure(call, object, context)
            .map(|measurement| (measurement, Citation::default()))
    }

    /// Measures `call` (one of [`Self::names`]) of `object`.
    ///
    /// # Errors
    ///
    /// As a property resolution: a missing service, an unreadable object, a
    /// measurement that cannot be made.
    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError>;

    /// Measures `call`, a value whose subject is a source
    /// ([`MeasuredSubject::Source`](axioval_ir::measured::MeasuredSubject::Source)),
    /// of `source`, with what it was measured against: what every object
    /// of the source reads, and what a template judging the source reads
    /// where it holds no object. By default it measures none.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn measure_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let _ = (source, context);
        Err(PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}` of a source",
            call.name()
        )))
    }

    /// Measures `call`, a value of the whole project
    /// ([`MeasuredSubject::Project`](axioval_ir::measured::MeasuredSubject::Project)),
    /// with what it was measured against: what every object reads, and
    /// what a template reads once per rule. By default it measures none.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let _ = context;
        Err(PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}` of the project",
            call.name()
        )))
    }

    /// The members `call`, a list whose subject is a source, lists of
    /// `source`, with the evidence of the measurement they come from. By
    /// default it lists none.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn members_of_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let _ = (source, context);
        Err(PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}` of a source",
            call.name()
        )))
    }

    /// Measures `call` of each of `objects`, one answer per object in
    /// order, each exactly what [`Self::measure`] answers for it: the batch
    /// entry point a run's resolver calls when a template reads one value of
    /// many objects. The default measures one object at a time; a provider
    /// overrides it to read its services and the call's parameters once, or
    /// to ask a service about every object together.
    fn measure_batch(
        &self,
        call: &MeasuredCall,
        objects: &[&ObjectId],
        context: &RuleContext<'_>,
    ) -> Vec<Result<Measurement, PropertyResolutionError>> {
        objects
            .iter()
            .map(|object| self.measure(call, object, context))
            .collect()
    }

    /// Whether the provider keeps what it measured for the run itself
    /// ([`MeasuredMemo`]), each value of it a cheap reading of that: a
    /// value read with bound arguments is then not kept a second time. By
    /// default it is kept.
    fn memoizes(&self) -> bool {
        false
    }
}

/// The providers of a run, by name, with the run's project and services.
#[derive(Clone)]
pub(crate) struct Providers {
    pub(crate) providers: Arc<Vec<Arc<dyn MeasuredProvider>>>,
    pub(crate) project: Arc<Project>,
}

impl Providers {
    /// The provider measuring `name`.
    pub(crate) fn of(&self, name: &str) -> Option<&Arc<dyn MeasuredProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.names().contains(&name))
    }
}

/// The members of `object` the measured member list `name` lists, as a run
/// measures them.
///
/// # Errors
///
/// An unknown or malformed list, a list no registered code measures, or a
/// measurement that cannot be made.
pub fn measured_members(
    services: &ServiceRegistry,
    object: &ObjectId,
    name: &str,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    measured_members_cited(services, object, name).map(|(members, _)| members)
}

/// [`measured_members`], with the evidence of the measurement the list
/// comes from.
///
/// # Errors
///
/// As [`measured_members`].
pub fn measured_members_cited(
    services: &ServiceRegistry,
    object: &ObjectId,
    name: &str,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
    let call = axioval_ir::measured::parse_members(name)
        .map_err(|error| PropertyResolutionError::Unavailable(error.to_string()))?;
    measured_members_bound(services, object, &call)
}

/// [`measured_members_cited`] of a list already parsed, its references
/// bound (`@name` to the rule's parameters, `@anchor` to the checked
/// object): what a template reads of a list naming the rule's parameters.
///
/// # Errors
///
/// As [`measured_members`].
pub fn measured_members_bound(
    services: &ServiceRegistry,
    object: &ObjectId,
    call: &axioval_ir::measured::MeasuredCall,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
    let providers = services.get::<Providers>().ok_or_else(|| {
        PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}` outside a run",
            call.name()
        ))
    })?;
    let provider = providers
        .providers
        .iter()
        .find(|provider| provider.member_lists().contains(&call.name()))
        .ok_or_else(|| {
            PropertyResolutionError::MissingService(format!(
                "no built-in code measures `{}`",
                call.name()
            ))
        })?;
    let context = RuleContext {
        project: &providers.project,
        services,
    };
    // A source's list is the list of each of its objects.
    if call.descriptor.subject == axioval_ir::measured::MeasuredSubject::Source {
        return provider.members_of_source(call, &object.source, &context);
    }
    provider.members_cited(call, object, &context)
}

/// What `provider` measures of `call` for `object`: of the object itself,
/// of its source or of the project, as the value's subject is declared,
/// with what it was measured against.
///
/// # Errors
///
/// As [`MeasuredProvider::measure`].
pub(crate) fn measure_subject(
    provider: &dyn MeasuredProvider,
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), PropertyResolutionError> {
    use axioval_ir::measured::MeasuredSubject;
    match call.descriptor.subject {
        MeasuredSubject::Object => provider.measure_cited(call, object, context),
        MeasuredSubject::Source => provider.measure_source(call, &object.source, context),
        MeasuredSubject::Project => provider.measure_project(call, context),
    }
}

/// One table of the run's memo: its entries by key, hashed by a fast keyed
/// hasher (each table seeded apart), since every value a template reads
/// looks one up.
type Table<K, V> = HashMap<K, V, foldhash::fast::RandomState>;

/// What providers measured in one run, shared by every rule and template
/// of it.
///
/// A provider answering several values from one measurement (a body's
/// directional extent, read as its length and as the positions of its two
/// ends) memoizes that measurement here, keyed by everything it depends on
/// (the object and the call's parameters, never a rule's), so the run takes
/// it once per object and parameter set however many rules and values read
/// it. Each run installs an empty one ([`crate::Runtime`] replaces any host
/// copy), so nothing measured in one run answers another; outside a run
/// there is none and a provider measures every time.
///
/// Entries are kept by key and value type: two providers using distinct
/// key types never share entries.
#[derive(Clone, Default)]
pub struct MeasuredMemo(Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send>>>>);

impl MeasuredMemo {
    /// The value memoized for `key`, or `measure`'s, memoized.
    ///
    /// `measure` runs without the memo locked, so it may read the memo
    /// itself; a measurement made twice meanwhile keeps the first.
    pub fn get_or_measure<K, V>(&self, key: K, measure: impl FnOnce() -> V) -> V
    where
        K: Hash + Eq + Send + 'static,
        V: Clone + Send + 'static,
    {
        let table = TypeId::of::<Table<K, V>>();
        if let Ok(entries) = self.0.lock()
            && let Some(value) = entries
                .get(&table)
                .and_then(|entries| entries.downcast_ref::<Table<K, V>>())
                .and_then(|entries| entries.get(&key))
        {
            return value.clone();
        }
        let value = measure();
        if let Ok(mut entries) = self.0.lock()
            && let Some(entries) = entries
                .entry(table)
                .or_insert_with(|| Box::new(Table::<K, V>::default()))
                .downcast_mut::<Table<K, V>>()
        {
            entries.entry(key).or_insert_with(|| value.clone());
        }
        value
    }

    /// The value memoized for `key`, if any.
    pub fn get<K, V>(&self, key: &K) -> Option<V>
    where
        K: Hash + Eq + Send + 'static,
        V: Clone + Send + 'static,
    {
        self.get_with(key, V::clone)
    }

    /// What `read` reads of the value memoized for `key`, if any, without
    /// copying the whole value. `read` runs with the memo locked, so it
    /// must not read the memo itself.
    pub fn get_with<K, V, R>(&self, key: &K, read: impl FnOnce(&V) -> R) -> Option<R>
    where
        K: Hash + Eq + Send + 'static,
        V: Send + 'static,
    {
        let entries = self.0.lock().ok()?;
        entries
            .get(&TypeId::of::<Table<K, V>>())?
            .downcast_ref::<Table<K, V>>()?
            .get(key)
            .map(read)
    }

    /// Memoizes `value` for `key`, replacing what was memoized: for a
    /// provider adding to what it measured of a key, such as one more axis
    /// of a body whose frame it measured already.
    pub fn insert<K, V>(&self, key: K, value: V)
    where
        K: Hash + Eq + Send + 'static,
        V: Clone + Send + 'static,
    {
        if let Ok(mut entries) = self.0.lock()
            && let Some(entries) = entries
                .entry(TypeId::of::<Table<K, V>>())
                .or_insert_with(|| Box::new(Table::<K, V>::default()))
                .downcast_mut::<Table<K, V>>()
        {
            entries.insert(key, value);
        }
    }

    /// Changes what is memoized for `key` (starting from the default) by
    /// `change`, in place: for a provider adding to what it measured of a
    /// key without copying it out and back. `change` runs with the memo
    /// locked, so it must not read the memo itself.
    pub fn update<K, V>(&self, key: K, change: impl FnOnce(&mut V))
    where
        K: Hash + Eq + Send + 'static,
        V: Default + Send + 'static,
    {
        if let Ok(mut entries) = self.0.lock()
            && let Some(entries) = entries
                .entry(TypeId::of::<Table<K, V>>())
                .or_insert_with(|| Box::new(Table::<K, V>::default()))
                .downcast_mut::<Table<K, V>>()
        {
            change(entries.entry(key).or_default());
        }
    }

    /// `measure` of `key` memoized in `services`' memo when a run installed
    /// one, else measured.
    pub fn of<K, V>(services: &ServiceRegistry, key: K, measure: impl FnOnce() -> V) -> V
    where
        K: Hash + Eq + Send + 'static,
        V: Clone + Send + 'static,
    {
        match services.get::<Self>() {
            Some(memo) => memo.get_or_measure(key, measure),
            None => measure(),
        }
    }
}

/// A measured call's bound arguments as a [`MeasuredMemo`] keys them:
/// every argument named (stated, defaulted or absent) with its value, in
/// the order named. Two keys are equal exactly when every argument is,
/// numbers by their bits and selections by their parameter and objects.
///
/// Its hash is computed once, when it is built, and it is cloned by
/// reference; it never formats the arguments, so a selection of many
/// objects costs its size and first object to hash.
#[derive(Clone, Debug)]
pub struct ArgumentsKey {
    hash: u64,
    arguments: Arc<[(&'static str, Option<MeasuredArgument>)]>,
}

impl ArgumentsKey {
    /// Every argument of `call`, by key.
    #[must_use]
    pub fn of(call: &MeasuredCall) -> Self {
        Self::from_parts(
            call.arguments
                .iter()
                .map(|(key, argument)| (*key, Some(argument.clone())))
                .collect(),
        )
    }

    /// The arguments `keys` of `call`, each `None` where the call has none.
    #[must_use]
    pub fn of_keys(call: &MeasuredCall, keys: &[&'static str]) -> Self {
        Self::from_parts(
            keys.iter()
                .map(|key| (*key, call.argument(key).cloned()))
                .collect(),
        )
    }

    fn from_parts(arguments: Arc<[(&'static str, Option<MeasuredArgument>)]>) -> Self {
        use std::hash::Hasher as _;
        // A fixed hasher: keys built apart hash alike.
        let mut hasher =
            std::hash::BuildHasher::build_hasher(&foldhash::fast::FixedState::default());
        for (key, argument) in arguments.iter() {
            key.hash(&mut hasher);
            match argument {
                Some(argument) => {
                    hasher.write_u8(1);
                    hash_argument(argument, &mut hasher);
                }
                None => hasher.write_u8(0),
            }
        }
        Self {
            hash: hasher.finish(),
            arguments,
        }
    }
}

/// `argument` hashed consistently with [`same_argument`].
fn hash_argument(argument: &MeasuredArgument, hasher: &mut impl std::hash::Hasher) {
    std::mem::discriminant(argument).hash(hasher);
    match argument {
        MeasuredArgument::Path(steps) => steps.hash(hasher),
        MeasuredArgument::SourceKind(text)
        | MeasuredArgument::Text(text)
        | MeasuredArgument::Parameter(text) => text.hash(hasher),
        MeasuredArgument::Length(value) | MeasuredArgument::Number(value) => {
            hasher.write_u64(value.to_bits());
        }
        MeasuredArgument::Choice(option) => option.hash(hasher),
        MeasuredArgument::Choices(options) => options.hash(hasher),
        MeasuredArgument::Vector(components) => {
            for component in components {
                hasher.write_u64(component.to_bits());
            }
        }
        MeasuredArgument::Property { set, name } => {
            set.hash(hasher);
            name.hash(hasher);
        }
        MeasuredArgument::Polygon(vertices) => {
            hasher.write_usize(vertices.len());
            for [lateral, up] in vertices {
                hasher.write_u64(lateral.to_bits());
                hasher.write_u64(up.to_bits());
            }
        }
        MeasuredArgument::Anchor => {}
        // A selection hashes by its parameter, its sizes and its first
        // object; equality compares every object.
        MeasuredArgument::Objects(selection) => {
            selection.parameter.hash(hasher);
            hasher.write_usize(selection.matched.len());
            selection.matched.first().hash(hasher);
            hasher.write_usize(selection.undecided.len());
            selection.undecided.first().hash(hasher);
        }
        // Rows hash by their count; equality compares them.
        MeasuredArgument::Table(rows) => hasher.write_usize(rows.len()),
        MeasuredArgument::Truth(value) => value.hash(hasher),
    }
}

/// Whether `a` and `b` are the same argument, numbers compared by their
/// bits so an argument is always the same as itself.
fn same_argument(a: &MeasuredArgument, b: &MeasuredArgument) -> bool {
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits();
    match (a, b) {
        (MeasuredArgument::Length(a), MeasuredArgument::Length(b))
        | (MeasuredArgument::Number(a), MeasuredArgument::Number(b)) => same(*a, *b),
        (MeasuredArgument::Vector(a), MeasuredArgument::Vector(b)) => {
            a.iter().zip(b).all(|(a, b)| same(*a, *b))
        }
        (MeasuredArgument::Polygon(a), MeasuredArgument::Polygon(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|([a0, a1], [b0, b1])| same(*a0, *b0) && same(*a1, *b1))
        }
        (a, b) => a == b,
    }
}

impl PartialEq for ArgumentsKey {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && (Arc::ptr_eq(&self.arguments, &other.arguments)
                || (self.arguments.len() == other.arguments.len()
                    && self.arguments.iter().zip(other.arguments.iter()).all(
                        |((key, a), (other_key, b))| {
                            key == other_key
                                && match (a, b) {
                                    (Some(a), Some(b)) => same_argument(a, b),
                                    (None, None) => true,
                                    _ => false,
                                }
                        },
                    )))
    }
}

impl Eq for ArgumentsKey {}

impl Hash for ArgumentsKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

/// Installs `providers` for a run over `project`, if there are any, with an
/// empty [`MeasuredMemo`].
pub(crate) fn install(
    services: &mut ServiceRegistry,
    providers: &[Arc<dyn MeasuredProvider>],
    project: &Project,
) {
    if providers.is_empty() {
        return;
    }
    services.replace(MeasuredMemo::default());
    services.replace(Providers {
        providers: Arc::new(providers.to_vec()),
        project: Arc::new(project.clone()),
    });
}
