//! Required members per group: a table of member entries, filled by a
//! maximum matching of the group's members. The matching stays here
//! (`compose`); the capability runs as a template judging what it found
//! (`group_composition/template.rs`).

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::CompositionMeasures;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::measured::MeasuredSelection;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::counts::{Population, relation_text};
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Traversal, Unavailable, invalid};
use crate::table_allocation::{
    KEY_COUNT, Key, KeyCells, KeyProperties, describe_keys, key_cells, key_properties, read_keys,
    row_name, test_keys, unknown_key,
};

/// The group cells, each over the group key of the same position.
pub(crate) const GROUP_COLUMNS: [&str; 3] = ["group", "group_2", "group_3"];

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("key_1", ColumnKind::TextPattern),
    TableColumn::optional("key_2", ColumnKind::TextPattern),
    TableColumn::optional("key_3", ColumnKind::TextPattern),
    TableColumn::optional("group", ColumnKind::TextPattern),
    TableColumn::optional("group_2", ColumnKind::TextPattern),
    TableColumn::optional("group_3", ColumnKind::TextPattern),
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("count", ColumnKind::Integer),
];

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("requirements", ParameterType::Table(COLUMNS)),
        ParameterDescriptor::optional("key_1", ParameterType::PropertyReference),
        ParameterDescriptor::optional("key_2", ParameterType::PropertyReference),
        ParameterDescriptor::optional("key_3", ParameterType::PropertyReference),
        ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ParameterDescriptor::optional("group_key", ParameterType::PropertyReference),
        ParameterDescriptor::optional("group_key_1", ParameterType::PropertyReference),
        ParameterDescriptor::optional("group_key_2", ParameterType::PropertyReference),
        ParameterDescriptor::optional("group_key_3", ParameterType::PropertyReference),
        ParameterDescriptor::optional("report_absent_groups", ParameterType::Boolean),
        ParameterDescriptor::optional("member_selector", ParameterType::Selector),
        ParameterDescriptor::optional("ungrouped_selector", ParameterType::Selector),
    ]
    .into_iter()
    .chain(crate::support::traversal_parameters())
    .collect()
}

/// Requires each selected group to hold a multiset of members: two
/// bedrooms, one kitchen and one bathroom per apartment.
///
/// The rule's selection are the groups. Each reaches its members through
/// the traversal parameters (`relationship` or `path`, required), such as
/// `IfcRelAssignsToGroup` or a derived relationship; `member_selector`
/// restricts which reached objects are members (every one by default).
///
/// The `requirements` table lists member entries. Its key cells `key_1` to
/// `key_3` are whole-value wildcard patterns over the member properties the
/// same-named parameters name, as in `table-allocation`; a member fits an
/// entry when every key cell the entry fills matches. `count` is how many
/// members the entry takes. A member fits any number of entries but fills
/// at most one place, so members are allocated to places by a maximum
/// bipartite matching: as many places as possible are filled, whatever
/// order members and entries are declared in.
///
/// With `group_key_1` (or `group_key`) to `group_key_3`, a row filling the
/// `group`, `group_2` and `group_3` cells applies only to groups whose
/// values of those keys it matches (a row without one applies to every
/// group), so one table carries every apartment type; a group that no row
/// with a group cell matches is a finding of its own. With
/// `report_absent_groups`, a row that no group in the model matches is a
/// project finding ("not in model"), unless a group whose selection or key
/// is undecided might match it.
///
/// Shortfalls and surpluses are reported only as far as every maximum
/// matching agrees: an entry misses members on its own when no allocation
/// could fill it, and entries that compete for the same members miss them
/// together. A group with a member whose selection or key is undecided is
/// not evaluated, never guessed. With `ungrouped_selector`, each object it
/// picks that no selected group reaches is a finding.
pub struct GroupComposition;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for GroupComposition {
    fn id(&self) -> &'static str {
        TEMPLATE.id
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

/// Checks the rule parameters `compositions` names, as the rule states
/// them: the declaration the capability refused, in its order and words.
///
/// # Errors
///
/// An invalid declaration.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, axioval_ir::contract::ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(stated.clone());
    Declaration::parse(&rule).map(|_| ())
}

/// What the matching found, each where its outcome goes.
pub(crate) enum Composed {
    /// An object or a group left open, for its reason.
    Open { at: ObjectId, why: Unavailable },
    /// A group no row with a group cell matches.
    Unmatched {
        group: ObjectId,
        keys: String,
        evidence: Vec<Evidence>,
    },
    /// A row no group in the model matches.
    Absent {
        name: String,
        evidence: Vec<Evidence>,
    },
    /// An object no selected group reaches.
    Ungrouped { object: ObjectId, via: String },
    /// Entries every maximum matching leaves short, together.
    Short {
        group: ObjectId,
        /// The entries as findings name them, with their verb.
        subject: String,
        filled: usize,
        places: usize,
        via: String,
        related: Vec<ObjectId>,
        evidence: Vec<Evidence>,
    },
    /// Members every maximum matching leaves without a place, together.
    Surplus {
        group: ObjectId,
        /// The entries they compete for as findings name them, with their
        /// verb; `None` for a member no entry fits.
        entries: Option<(String, &'static str)>,
        /// The key values of a member no entry fits.
        keys: String,
        found: usize,
        places: usize,
        via: String,
        related: Vec<ObjectId>,
        evidence: Vec<Evidence>,
    },
}

/// The members, groups and ungrouped objects measured values' arguments
/// bound.
pub(crate) struct Selected<'s> {
    pub(crate) groups: &'s MeasuredSelection,
    pub(crate) members: &'s Population,
    pub(crate) ungrouped: Option<&'s MeasuredSelection>,
}

/// Matches each group's members to the rows of `rule`, and lists what the
/// template judges, in the capability's order.
///
/// # Errors
///
/// An invalid declaration.
pub(crate) fn compose(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    selected: &Selected<'_>,
) -> Result<Vec<Composed>, Unavailable> {
    let declaration = Declaration::parse(rule)?;
    let members = selected.members;
    let ungrouped = selected.ungrouped.map(|picked| Population {
        matched: picked.matched.clone(),
        undecided: picked.undecided.clone(),
        first: None,
    });
    let universe: Vec<&Object> = context
        .project
        .objects()
        .filter(|object| {
            members.contains(&object.id)
                || ungrouped
                    .as_ref()
                    .is_some_and(|ungrouped| ungrouped.contains(&object.id))
        })
        .collect();
    // The groups in the order they are selected: the model's objects, then
    // the run's resource objects and derived groups.
    let in_order = |ids: &std::collections::BTreeSet<ObjectId>| -> Vec<&Object> {
        let mut objects: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| ids.contains(&object.id))
            .collect();
        objects.extend(
            ids.iter()
                .filter(|id| context.project.object(id).is_none())
                .filter_map(|id| crate::selection::object_by_id(context, id)),
        );
        objects
    };
    let groups: Vec<&Object> = in_order(&selected.groups.matched);
    let undecided_groups: Vec<ObjectId> = in_order(&selected.groups.undecided)
        .into_iter()
        .map(|object| object.id.clone())
        .collect();
    let mut found = Vec::new();
    if declaration.report_absent {
        absent(
            context,
            &declaration,
            &groups,
            &undecided_groups,
            &mut found,
        );
    }
    let mut judge = Judge {
        context,
        declaration: &declaration,
        members,
        keys: BTreeMap::new(),
        reported: BTreeSet::new(),
        found: &mut found,
    };
    // Objects some decided group reaches, and whether every group could be
    // walked.
    let mut grouped = BTreeSet::new();
    let mut complete = true;
    for group in groups {
        match declaration.traversal.related(context, &group.id, &universe) {
            Ok((reached, evidence)) => {
                judge.group(group, &reached, evidence);
                grouped.extend(reached);
            }
            Err(why) => {
                complete = false;
                judge.found.push(Composed::Open {
                    at: group.id.clone(),
                    why,
                });
            }
        }
    }
    if let Some(ungrouped) = ungrouped {
        let reach = Reach {
            universe: &universe,
            grouped: &grouped,
            undecided_groups: &undecided_groups,
            complete,
        };
        outside(context, &declaration, &ungrouped, &reach, &mut found);
    }
    Ok(found)
}

/// Each requirement row no group in the model matches, unless a group of
/// undecided selection or key might.
fn absent(
    context: &RuleContext<'_>,
    declaration: &Declaration<'_>,
    groups: &[&Object],
    undecided_groups: &[ObjectId],
    found: &mut Vec<Composed>,
) {
    let rows = declaration.rows.len();
    let (mut present, mut maybe) = (vec![false; rows], vec![false; rows]);
    let mut evidence = Vec::new();
    let decided = groups.iter().map(|group| (*group, true));
    let undecided = undecided_groups
        .iter()
        .filter_map(|id| context.project.object(id))
        .map(|group| (group, false));
    for (group, sure) in decided.chain(undecided) {
        let (tests, _, cited) = declaration.group_tests(context, group);
        evidence.extend(cited);
        for (row, test) in tests.into_iter().enumerate() {
            match test {
                RowTest::Match(_) if sure => present[row] = true,
                RowTest::NoMatch => {}
                _ => maybe[row] = true,
            }
        }
    }
    for (index, row) in declaration.rows.iter().enumerate() {
        if present[index] {
            continue;
        }
        let name = row_name(row.number, row.label, &row.group, &declaration.group_key);
        if maybe[index] {
            found.push(Composed::Open {
                at: axioval_engine::template::scope_stand_in(None),
                why: (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "group-composition: whether a group matches {name} is undecided: a \
                         group's selection or key cannot be read"
                    ),
                ),
            });
        } else {
            found.push(Composed::Absent {
                name,
                evidence: evidence.clone(),
            });
        }
    }
}

/// What the decided groups reach, and which groups are undecided.
struct Reach<'r> {
    universe: &'r [&'r Object],
    grouped: &'r BTreeSet<ObjectId>,
    undecided_groups: &'r [ObjectId],
    /// Whether every decided group could be walked.
    complete: bool,
}

/// Each `ungrouped` object no group reaches, unless a group that could not
/// be decided or walked might.
fn outside(
    context: &RuleContext<'_>,
    declaration: &Declaration<'_>,
    ungrouped: &Population,
    reach: &Reach<'_>,
    found: &mut Vec<Composed>,
) {
    let mut complete = reach.complete;
    // Objects a group of undecided selection reaches.
    let mut maybe_grouped = BTreeSet::new();
    for group in reach.undecided_groups {
        let Some(object) = context.project.object(group) else {
            complete = false;
            continue;
        };
        match declaration
            .traversal
            .related(context, &object.id, reach.universe)
        {
            Ok((reached, _)) => maybe_grouped.extend(reached),
            Err(_) => complete = false,
        }
    }
    let via = relation_text(Some(&declaration.traversal));
    for object in ungrouped.matched.union(&ungrouped.undecided) {
        if reach.grouped.contains(object) {
            continue;
        }
        let decided = ungrouped.matched.contains(object);
        if decided && complete && !maybe_grouped.contains(object) {
            found.push(Composed::Ungrouped {
                object: object.clone(),
                via: via.clone(),
            });
        } else {
            let why = if decided {
                "a group that cannot be decided or walked may hold it"
            } else {
                "whether it must be in a group is undecided"
            };
            found.push(Composed::Open {
                at: object.clone(),
                why: (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("group-composition: no group reaches it {via}, but {why}"),
                ),
            });
        }
    }
}

/// One requirement row as declared.
struct Row<'a> {
    /// One-based, as a reviewer counts.
    number: usize,
    label: Option<&'a str>,
    keys: KeyCells<'a>,
    /// The `group`, `group_2` and `group_3` cells, as key cells over the
    /// group keys.
    group: KeyCells<'a>,
    count: usize,
}

struct Declaration<'a> {
    rows: Vec<Row<'a>>,
    properties: KeyProperties<'a>,
    group_key: KeyProperties<'a>,
    report_absent: bool,
    traversal: Traversal,
}

impl<'a> Declaration<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
        let properties = key_properties(&parameters)?;
        let group_key = match (
            parameters.property("group_key")?,
            parameters.property("group_key_1")?,
        ) {
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "`group_key` and `group_key_1` name one key; declare one",
                ));
            }
            (first, other) => [
                first.or(other),
                parameters.property("group_key_2")?,
                parameters.property("group_key_3")?,
                None,
            ],
        };
        let traversal = parameters.traversal()?.ok_or_else(|| {
            invalid("a group reaches its members only through `relationship` or `path`")
        })?;
        let table = parameters
            .table("requirements")?
            .ok_or_else(|| invalid("parameter `requirements` is required"))?;
        if table.is_empty() {
            return Err(invalid("`requirements` has no row"));
        }
        let mut rows = Vec::new();
        for (index, row) in table.into_iter().enumerate() {
            let number = index + 1;
            let keys = key_cells(row, number, &properties, case_sensitive)?;
            let mut group = [None, None, None, None];
            for (slot, (column, key)) in group.iter_mut().zip(GROUP_COLUMNS.iter().zip(&group_key))
            {
                let Some(text) = row.text(column)? else {
                    continue;
                };
                if key.is_none() {
                    return Err(invalid(format!(
                        "row {number} fills `{column}`, but no group key property is declared \
                         for it"
                    )));
                }
                *slot = Some((
                    row.pattern(column, case_sensitive)?
                        .expect("a filled cell compiles"),
                    text,
                ));
            }
            let count = row
                .integer("count")?
                .ok_or_else(|| invalid(format!("row {number} has no count")))?;
            let count = usize::try_from(count)
                .map_err(|_| invalid(format!("row {number} has a negative count")))?;
            rows.push(Row {
                number,
                label: row.text("label")?,
                keys,
                group,
                count,
            });
        }
        for (at, column) in GROUP_COLUMNS.iter().enumerate() {
            if group_key[at].is_some() && rows.iter().all(|row| row.group[at].is_none()) {
                return Err(invalid(format!(
                    "a group key is declared for `{column}`, but no row fills it"
                )));
            }
        }
        // Read as the capability read them, so one of another type is
        // refused in its order.
        let report_absent = parameters.boolean("report_absent_groups")?.unwrap_or(false);
        parameters.selector("member_selector")?;
        parameters.selector("ungrouped_selector")?;
        Ok(Self {
            rows,
            properties,
            group_key,
            report_absent,
            traversal,
        })
    }

    /// Whether each row applies to `group`: a row without group cells
    /// always does. With the group's key values and the evidence for them.
    fn group_tests(
        &self,
        context: &RuleContext<'_>,
        group: &Object,
    ) -> (Vec<RowTest>, [Option<Key>; KEY_COUNT], Vec<Evidence>) {
        let used = [0, 1, 2, 3].map(|index| self.rows.iter().any(|row| row.group[index].is_some()));
        let (keys, cited) = read_keys(context, group, &self.group_key, used);
        let tests = self
            .rows
            .iter()
            .map(|row| {
                if row.group.iter().all(Option::is_none) {
                    RowTest::Match(0)
                } else {
                    test_keys(&row.group, &keys)
                }
            })
            .collect();
        (tests, keys, cited)
    }

    fn name(&self, row: usize) -> String {
        let row = &self.rows[row];
        row_name(row.number, row.label, &row.keys, &self.properties)
    }

    /// Several rows as a reviewer reads them: `a`, `b` and `c`.
    fn names(&self, rows: &[usize]) -> String {
        let names: Vec<String> = rows.iter().map(|row| self.name(*row)).collect();
        match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            _ => names.concat(),
        }
    }
}

/// A member's key values, read once however many groups hold it.
struct MemberKeys {
    keys: [Option<Key>; KEY_COUNT],
    evidence: Vec<Evidence>,
}

/// Matches one group after another.
struct Judge<'j, 'a> {
    context: &'j RuleContext<'j>,
    declaration: &'j Declaration<'a>,
    members: &'j Population,
    keys: BTreeMap<ObjectId, MemberKeys>,
    /// Members already reported not evaluated for an unreadable key.
    reported: BTreeSet<ObjectId>,
    found: &'j mut Vec<Composed>,
}

impl Judge<'_, '_> {
    fn open(&mut self, group: &ObjectId, reason: NotEvaluatedReason, message: &str) {
        self.found.push(Composed::Open {
            at: group.clone(),
            why: (reason, format!("group-composition: {message}")),
        });
    }

    /// The requirement rows that apply to `group`, or `None` when the group
    /// is reported instead.
    fn entries(&mut self, group: &Object, evidence: &mut Vec<Evidence>) -> Option<Vec<usize>> {
        let declaration = self.declaration;
        let rows = &declaration.rows;
        if declaration.group_key.iter().all(Option::is_none) {
            return Some((0..rows.len()).collect());
        }
        let (tests, keys, cited) = declaration.group_tests(self.context, group);
        evidence.extend(cited);
        let indexed: Vec<usize> = (0..rows.len()).collect();
        match match_rows(&indexed, RowSelection::All, |index| tests[*index]) {
            Matched::Rows(matched) => {
                if matched
                    .iter()
                    .all(|(index, _)| rows[*index].group.iter().all(Option::is_none))
                {
                    self.found.push(Composed::Unmatched {
                        group: group.id.clone(),
                        keys: describe_keys(&declaration.group_key, &keys),
                        evidence: std::mem::take(evidence),
                    });
                    return None;
                }
                Some(matched.into_iter().map(|(index, _)| index).collect())
            }
            Matched::Undecided | Matched::Ambiguous(_) => {
                let (reason, message) = unknown_key(&keys);
                self.open(
                    &group.id,
                    reason,
                    &format!("the group's requirements cannot be decided: {message}"),
                );
                None
            }
        }
    }

    fn group(&mut self, group: &Object, reached: &[ObjectId], mut evidence: Vec<Evidence>) {
        let Some(entries) = self.entries(group, &mut evidence) else {
            return;
        };
        let via = relation_text(Some(&self.declaration.traversal));
        let members: Vec<&ObjectId> = reached
            .iter()
            .filter(|id| self.members.matched.contains(*id))
            .collect();
        let undecided = reached.len() - members.len();
        if undecided > 0 {
            self.open(
                &group.id,
                NotEvaluatedReason::IncompleteEvidence,
                &format!("{undecided} object(s) it reaches {via} may be members"),
            );
            return;
        }
        let Some(fits) = self.fits(group, &members, &entries, &via) else {
            return;
        };
        let capacity: Vec<usize> = entries
            .iter()
            .map(|row| self.declaration.rows[*row].count)
            .collect();
        let allocation = Allocation::maximum(&fits, &capacity);
        for part in allocation.shortfalls() {
            let rows: Vec<usize> = part.entries.iter().map(|entry| entries[*entry]).collect();
            let places: usize = part.entries.iter().map(|entry| capacity[*entry]).sum();
            let subject = if rows.len() == 1 {
                format!("{} has", self.declaration.name(rows[0]))
            } else {
                format!("{} together have", self.declaration.names(&rows))
            };
            let (related, evidence) =
                self.cited(&evidence, part.members.iter().map(|m| members[*m]));
            self.found.push(Composed::Short {
                group: group.id.clone(),
                subject,
                filled: part.members.len(),
                places,
                via: via.clone(),
                related,
                evidence,
            });
        }
        for part in allocation.surpluses() {
            let (keys, entries_named) = if part.entries.is_empty() {
                let member = members[part.members[0]];
                let shown = describe_keys(&self.declaration.properties, &self.keys_of(member).keys);
                (shown, None)
            } else {
                let rows: Vec<usize> = part.entries.iter().map(|entry| entries[*entry]).collect();
                let verb = if rows.len() == 1 { "takes" } else { "take" };
                (String::new(), Some((self.declaration.names(&rows), verb)))
            };
            let places: usize = part.entries.iter().map(|entry| capacity[*entry]).sum();
            let (related, evidence) =
                self.cited(&evidence, part.members.iter().map(|m| members[*m]));
            self.found.push(Composed::Surplus {
                group: group.id.clone(),
                entries: entries_named,
                keys,
                found: part.members.len(),
                places,
                via: via.clone(),
                related,
                evidence,
            });
        }
    }

    /// Which entries each member fits, as positions in `entries`; `None`
    /// when a key leaves a fit undecided and the group is reported instead.
    fn fits(
        &mut self,
        group: &Object,
        members: &[&ObjectId],
        entries: &[usize],
        via: &str,
    ) -> Option<Vec<Vec<usize>>> {
        let declaration = self.declaration;
        let mut fits = Vec::with_capacity(members.len());
        let mut unknown = Vec::new();
        for member in members {
            let keys = self.keys_of(member);
            let mut fit = Vec::new();
            let mut undecided = false;
            for (position, row) in entries.iter().enumerate() {
                match test_keys(&declaration.rows[*row].keys, &keys.keys) {
                    RowTest::Match(_) => fit.push(position),
                    RowTest::NoMatch => {}
                    RowTest::Undecided => undecided = true,
                }
            }
            if undecided {
                unknown.push(((*member).clone(), unknown_key(&keys.keys)));
            }
            fits.push(fit);
        }
        if let Some((_, (reason, _))) = unknown.first() {
            let reason = reason.clone();
            let count = unknown.len();
            for (member, (reason, message)) in unknown {
                if self.reported.insert(member.clone()) {
                    self.found.push(Composed::Open {
                        at: member,
                        why: (
                            reason,
                            format!(
                                "group-composition: which entries it fits is undecided: {message}"
                            ),
                        ),
                    });
                }
            }
            self.open(
                &group.id,
                reason,
                &format!("{count} member(s) {via} may fit entries their keys cannot decide"),
            );
            return None;
        }
        Some(fits)
    }

    fn keys_of(&mut self, member: &ObjectId) -> &MemberKeys {
        let (context, declaration) = (self.context, self.declaration);
        self.keys.entry(member.clone()).or_insert_with(|| {
            let used = [0, 1, 2, 3]
                .map(|index| declaration.rows.iter().any(|row| row.keys[index].is_some()));
            match context.project.object(member) {
                Some(object) => {
                    let (keys, evidence) =
                        read_keys(context, object, &declaration.properties, used);
                    MemberKeys { keys, evidence }
                }
                None => MemberKeys {
                    keys: used.map(|used| {
                        used.then(|| {
                            Key::Unknown(
                                NotEvaluatedReason::InvalidEvidence,
                                "the member is not in the project".into(),
                            )
                        })
                    }),
                    evidence: Vec::new(),
                },
            }
        })
    }

    /// The objects a finding relates and what it cites: the group's
    /// evidence and each related member's keys.
    fn cited<'m>(
        &mut self,
        evidence: &[Evidence],
        related: impl Iterator<Item = &'m ObjectId>,
    ) -> (Vec<ObjectId>, Vec<Evidence>) {
        let related: Vec<ObjectId> = related.cloned().collect();
        let mut evidence = evidence.to_vec();
        for member in &related {
            evidence.extend(self.keys_of(member).evidence.iter().cloned());
        }
        (related, evidence)
    }
}

/// Members allocated to entries by a maximum matching; indices are
/// positions in the member and entry lists.
struct Allocation<'f> {
    fits: &'f [Vec<usize>],
    capacity: &'f [usize],
    /// The entry each member fills, if any.
    assigned: Vec<Option<usize>>,
}

/// Entries and members whose outcome every maximum matching shares.
struct Part {
    entries: Vec<usize>,
    members: Vec<usize>,
}

impl<'f> Allocation<'f> {
    /// A maximum allocation: each member in turn takes a free place along
    /// the shortest augmenting path, moving members already placed.
    /// A member that finds no path now never will, so one pass suffices.
    fn maximum(fits: &'f [Vec<usize>], capacity: &'f [usize]) -> Self {
        let mut assigned: Vec<Option<usize>> = vec![None; fits.len()];
        let mut load = vec![0_usize; capacity.len()];
        for start in 0..fits.len() {
            // `reached_from[e]`: the member whose fit led to entry `e`.
            let mut reached_from: Vec<Option<usize>> = vec![None; capacity.len()];
            let mut seen = vec![false; fits.len()];
            seen[start] = true;
            let mut queue = VecDeque::from([start]);
            let mut free = None;
            'search: while let Some(member) = queue.pop_front() {
                for &entry in &fits[member] {
                    if reached_from[entry].is_some() {
                        continue;
                    }
                    reached_from[entry] = Some(member);
                    if load[entry] < capacity[entry] {
                        free = Some(entry);
                        break 'search;
                    }
                    for (other, placed) in assigned.iter().enumerate() {
                        if *placed == Some(entry) && !seen[other] {
                            seen[other] = true;
                            queue.push_back(other);
                        }
                    }
                }
            }
            let Some(free) = free else { continue };
            load[free] += 1;
            let mut entry = free;
            loop {
                let member = reached_from[entry].expect("every reached entry has a member");
                let previous = assigned[member].replace(entry);
                match previous {
                    Some(previous) if member != start => entry = previous,
                    _ => break,
                }
            }
        }
        Self {
            fits,
            capacity,
            assigned,
        }
    }

    fn load(&self, entry: usize) -> usize {
        self.assigned
            .iter()
            .filter(|placed| **placed == Some(entry))
            .count()
    }

    /// Entries that some maximum allocation leaves short, split into parts
    /// that compete for the same members: each part's shortfall is its
    /// places less its members, whichever maximum allocation is taken.
    fn shortfalls(&self) -> Vec<Part> {
        let entries = self.capacity.len();
        // An entry can be left short when a member it holds can move to an
        // entry that can be left short.
        let mut short: Vec<bool> = (0..entries)
            .map(|entry| self.load(entry) < self.capacity[entry])
            .collect();
        loop {
            let mut grew = false;
            for (member, placed) in self.assigned.iter().enumerate() {
                let Some(placed) = *placed else { continue };
                if !short[placed] && self.fits[member].iter().any(|entry| short[*entry]) {
                    short[placed] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        // Every member fitting a short entry fills one.
        self.parts(
            |entry| short[entry],
            |member| self.assigned[member].is_some_and(|entry| short[entry]),
        )
    }

    /// Members that some maximum allocation leaves without a place, split
    /// into parts that compete for the same entries: each part's surplus is
    /// its members less its places.
    fn surpluses(&self) -> Vec<Part> {
        let mut spare: Vec<bool> = self.assigned.iter().map(Option::is_none).collect();
        loop {
            let mut grew = false;
            for (member, placed) in self.assigned.iter().enumerate() {
                let Some(placed) = *placed else { continue };
                if !spare[member]
                    && (0..self.fits.len())
                        .any(|other| spare[other] && self.fits[other].contains(&placed))
                {
                    spare[member] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        // Every entry a spare member fits is full of spare members.
        let full = |entry: usize| {
            (0..self.fits.len()).any(|member| spare[member] && self.fits[member].contains(&entry))
        };
        self.parts(full, |member| spare[member])
    }

    /// The connected parts of the fit graph restricted to the entries and
    /// members given, each with at least one entry or member; in order of
    /// their first entry, then of their first member.
    fn parts(
        &self,
        entry_in: impl Fn(usize) -> bool,
        member_in: impl Fn(usize) -> bool,
    ) -> Vec<Part> {
        let entries = self.capacity.len();
        let members = self.fits.len();
        // Nodes: entries first, then members.
        let mut part_of: Vec<Option<usize>> = vec![None; entries + members];
        let mut parts = Vec::new();
        let included = |node: usize| {
            if node < entries {
                entry_in(node)
            } else {
                member_in(node - entries)
            }
        };
        for start in 0..entries + members {
            if part_of[start].is_some() || !included(start) {
                continue;
            }
            let index = parts.len();
            let mut part = Part {
                entries: Vec::new(),
                members: Vec::new(),
            };
            part_of[start] = Some(index);
            let mut stack = vec![start];
            while let Some(node) = stack.pop() {
                let neighbours: Vec<usize> = if node < entries {
                    part.entries.push(node);
                    (0..members)
                        .filter(|member| self.fits[*member].contains(&node))
                        .map(|member| entries + member)
                        .collect()
                } else {
                    part.members.push(node - entries);
                    self.fits[node - entries].clone()
                };
                for next in neighbours {
                    if part_of[next].is_none() && included(next) {
                        part_of[next] = Some(index);
                        stack.push(next);
                    }
                }
            }
            part.entries.sort_unstable();
            part.members.sort_unstable();
            parts.push(part);
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::Allocation;

    fn placed(fits: &[Vec<usize>], capacity: &[usize]) -> usize {
        Allocation::maximum(fits, capacity)
            .assigned
            .iter()
            .flatten()
            .count()
    }

    #[test]
    fn the_matching_moves_placed_members_along_augmenting_paths() {
        // Member 0 fits both entries, member 1 only the first: greedy
        // placement of member 0 in the first entry strands member 1.
        assert_eq!(placed(&[vec![0, 1], vec![0]], &[1, 1]), 2);
        // A longer chain: 0 → {0,1}, 1 → {1,2}, 2 → {0}.
        assert_eq!(placed(&[vec![0, 1], vec![1, 2], vec![0]], &[1, 1, 1]), 3);
        // Capacities above one, and an entry of capacity zero.
        assert_eq!(placed(&[vec![0, 1], vec![0], vec![0], vec![1]], &[2, 0]), 2);
    }

    /// The largest allocation, by trying every one.
    fn brute_force(fits: &[Vec<usize>], capacity: &[usize]) -> usize {
        fn go(fits: &[Vec<usize>], load: &mut Vec<usize>, capacity: &[usize]) -> usize {
            let Some((first, rest)) = fits.split_first() else {
                return 0;
            };
            let mut best = go(rest, load, capacity);
            for &entry in first {
                if load[entry] < capacity[entry] {
                    load[entry] += 1;
                    best = best.max(1 + go(rest, load, capacity));
                    load[entry] -= 1;
                }
            }
            best
        }
        go(fits, &mut vec![0; capacity.len()], capacity)
    }

    #[test]
    fn the_matching_is_maximum_and_its_parts_account_for_every_shortfall_and_surplus() {
        let subsets = [
            vec![],
            vec![0],
            vec![1],
            vec![0, 1],
            vec![2],
            vec![0, 2],
            vec![1, 2],
        ];
        for a in &subsets {
            for b in &subsets {
                for c in &subsets {
                    for capacity in [[1, 1, 1], [2, 0, 1], [0, 2, 2], [1, 2, 0]] {
                        let fits = [a.clone(), b.clone(), c.clone()];
                        let allocation = Allocation::maximum(&fits, &capacity);
                        let allocated = allocation.assigned.iter().flatten().count();
                        assert_eq!(
                            allocated,
                            brute_force(&fits, &capacity),
                            "{fits:?} {capacity:?}"
                        );
                        let missing: usize = allocation
                            .shortfalls()
                            .iter()
                            .map(|part| {
                                let places: usize =
                                    part.entries.iter().map(|entry| capacity[*entry]).sum();
                                places - part.members.len()
                            })
                            .sum();
                        assert_eq!(missing, capacity.iter().sum::<usize>() - allocated);
                        let surplus: usize = allocation
                            .surpluses()
                            .iter()
                            .map(|part| {
                                let places: usize =
                                    part.entries.iter().map(|entry| capacity[*entry]).sum();
                                part.members.len() - places
                            })
                            .sum();
                        assert_eq!(surplus, fits.len() - allocated);
                    }
                }
            }
        }
    }

    #[test]
    fn shortfalls_and_surpluses_are_split_into_parts_every_maximum_matching_shares() {
        // One member fits two entries of one place each: which one is short
        // is not decided, so both are short together.
        let (fits, capacity) = ([vec![0, 1]], [1, 1]);
        let allocation = Allocation::maximum(&fits, &capacity);
        let shortfalls = allocation.shortfalls();
        assert_eq!(shortfalls.len(), 1);
        assert_eq!(shortfalls[0].entries, [0, 1]);
        assert_eq!(shortfalls[0].members, [0]);
        assert!(allocation.surpluses().is_empty());

        // Three members for one place, and a member fitting nothing.
        let (fits, capacity) = ([vec![0], vec![0], vec![0], vec![]], [1]);
        let allocation = Allocation::maximum(&fits, &capacity);
        assert!(allocation.shortfalls().is_empty());
        let surpluses = allocation.surpluses();
        assert_eq!(surpluses.len(), 2);
        assert_eq!(
            (surpluses[0].entries.len(), surpluses[0].members.len()),
            (1, 3)
        );
        assert_eq!(
            (surpluses[1].entries.len(), surpluses[1].members.clone()),
            (0, vec![3])
        );
    }
}
