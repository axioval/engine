//! `group-composition` as it judged before its decision became a template
//! over its allocation (#286), kept to hold the template to (see
//! `templates.md`). Compiled only with the `parity-reference` feature; never
//! registered.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, Scope};

use crate::counts::{Population, relation_text};
use crate::pairs::severity;
use crate::selection::select_objects;
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};
use crate::table_allocation::{
    KEY_COUNT, Key, KeyCells, KeyProperties, describe_keys, key_cells, key_properties, read_keys,
    row_name, test_keys, unknown_key,
};

use super::{Allocation, GROUP_COLUMNS};

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

impl RuleCapability for GroupComposition {
    fn id(&self) -> &'static str {
        "axioval:capability.group-composition"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declaration = match Declaration::parse(rule) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("group-composition: {message}"),
                );
            }
        };
        let members = Population::of(context, declaration.members);
        let ungrouped = declaration
            .ungrouped
            .map(|selector| Population::of(context, selector));
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
        let (groups, mut evaluation) = select_objects(context, &rule.selector);
        let undecided_groups: Vec<ObjectId> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();

        if declaration.report_absent {
            report_absent(
                context,
                rule,
                &declaration,
                &groups,
                &undecided_groups,
                &mut evaluation,
            );
        }
        let mut judge = Judge {
            context,
            rule,
            declaration: &declaration,
            members: &members,
            keys: BTreeMap::new(),
            reported: BTreeSet::new(),
            evaluation: &mut evaluation,
        };
        // Objects some decided group reaches, and whether every group could be walked.
        let mut grouped = BTreeSet::new();
        let mut complete = true;
        for group in groups {
            match declaration.traversal.related(context, &group.id, &universe) {
                Ok((reached, evidence)) => {
                    judge.group(group, &reached, evidence);
                    grouped.extend(reached);
                }
                Err((reason, message)) => {
                    complete = false;
                    judge
                        .evaluation
                        .push_object_not_evaluated(group.id.clone(), reason, message);
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
            report_ungrouped(
                context,
                rule,
                &declaration,
                &ungrouped,
                &reach,
                &mut evaluation,
            );
        }
        evaluation
    }
}

/// A project finding for each requirement row no group in the model
/// matches, unless a group of undecided selection or key might.
fn report_absent(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declaration: &Declaration<'_>,
    groups: &[&Object],
    undecided_groups: &[ObjectId],
    evaluation: &mut CapabilityEvaluation,
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
            evaluation.push_not_evaluated(
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "group-composition: whether a group matches {name} is undecided: a group's \
                     selection or key cannot be read"
                ),
            );
        } else {
            evaluation.push_finding(
                Finding::new(
                    rule.id.clone(),
                    Scope::Project,
                    severity(rule),
                    format!("not in model: no group matches {name}"),
                )
                .with_evidence(evidence.clone()),
            );
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

/// A finding for each `ungrouped` object no group reaches, unless a group
/// that could not be decided or walked might.
fn report_ungrouped(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declaration: &Declaration<'_>,
    ungrouped: &Population,
    reach: &Reach<'_>,
    evaluation: &mut CapabilityEvaluation,
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
            evaluation.push_finding(finding(
                rule,
                object,
                format!("in no group {via}"),
                Vec::new(),
                Vec::new(),
            ));
        } else {
            let why = if decided {
                "a group that cannot be decided or walked may hold it"
            } else {
                "whether it must be in a group is undecided"
            };
            evaluation.push_object_not_evaluated(
                object.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!("group-composition: no group reaches it {via}, but {why}"),
            );
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
    members: &'a Selector,
    ungrouped: Option<&'a Selector>,
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
        Ok(Self {
            rows,
            properties,
            group_key,
            report_absent: parameters.boolean("report_absent_groups")?.unwrap_or(false),
            members: parameters
                .selector("member_selector")?
                .unwrap_or(&Selector::All),
            ungrouped: parameters.selector("ungrouped_selector")?,
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

/// Judges one group after another.
struct Judge<'j, 'a> {
    context: &'j RuleContext<'j>,
    rule: &'j CompiledRule,
    declaration: &'j Declaration<'a>,
    members: &'j Population,
    keys: BTreeMap<ObjectId, MemberKeys>,
    /// Members already reported not evaluated for an unreadable key.
    reported: BTreeSet<ObjectId>,
    evaluation: &'j mut CapabilityEvaluation,
}

impl Judge<'_, '_> {
    fn not_evaluated(&mut self, group: &ObjectId, reason: NotEvaluatedReason, message: &str) {
        self.evaluation.push_object_not_evaluated(
            group.clone(),
            reason,
            format!("group-composition: {message}"),
        );
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
                    let shown = describe_keys(&declaration.group_key, &keys);
                    self.evaluation.push_finding(finding(
                        self.rule,
                        &group.id,
                        format!("no requirement row matches the group ({shown})"),
                        std::mem::take(evidence),
                        Vec::new(),
                    ));
                    return None;
                }
                Some(matched.into_iter().map(|(index, _)| index).collect())
            }
            Matched::Undecided | Matched::Ambiguous(_) => {
                let (reason, message) = unknown_key(&keys);
                self.not_evaluated(
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
            self.not_evaluated(
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
            let filled = part.members.len();
            let subject = if rows.len() == 1 {
                format!("{} has", self.declaration.name(rows[0]))
            } else {
                format!("{} together have", self.declaration.names(&rows))
            };
            let message = format!(
                "{subject} {filled} of {places} required member(s) {via}; {} missing",
                places - filled
            );
            self.report(
                group,
                message,
                &evidence,
                part.members.iter().map(|m| members[*m]),
            );
        }
        for part in allocation.surpluses() {
            let found = part.members.len();
            let message = if part.entries.is_empty() {
                let member = members[part.members[0]];
                let shown = describe_keys(&self.declaration.properties, &self.keys_of(member).keys);
                format!("surplus member {via}: no entry fits it ({shown})")
            } else {
                let rows: Vec<usize> = part.entries.iter().map(|entry| entries[*entry]).collect();
                let places: usize = part.entries.iter().map(|entry| capacity[*entry]).sum();
                let verb = if rows.len() == 1 { "takes" } else { "take" };
                format!(
                    "{} {verb} {places} member(s), but {found} fit {via}; {} surplus",
                    self.declaration.names(&rows),
                    found - places
                )
            };
            self.report(
                group,
                message,
                &evidence,
                part.members.iter().map(|m| members[*m]),
            );
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
                    self.evaluation.push_object_not_evaluated(
                        member,
                        reason,
                        format!("group-composition: which entries it fits is undecided: {message}"),
                    );
                }
            }
            self.not_evaluated(
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

    fn report<'m>(
        &mut self,
        group: &Object,
        message: String,
        evidence: &[Evidence],
        related: impl Iterator<Item = &'m ObjectId>,
    ) {
        let related: Vec<ObjectId> = related.cloned().collect();
        let mut evidence = evidence.to_vec();
        for member in &related {
            evidence.extend(self.keys_of(member).evidence.iter().cloned());
        }
        self.evaluation
            .push_finding(finding(self.rule, &group.id, message, evidence, related));
    }
}
