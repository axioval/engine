//! A clash matrix: tolerances and severities per pair of categories.
//!
//! Each candidate pair is judged with the tolerance profile of the one matrix
//! cell that covers it most specifically. A cell keys each side of the pair
//! by category: the discipline the object's source plays, key properties and
//! a selector, each optional. The pair judgement itself (classes, interval
//! handling, exclusions, measurement) is `clash`'s, shared through
//! [`crate::clash`], so a cell means exactly what a `clash` rule with the
//! same values means.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, ProximityProjection, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, Severity};

use crate::clash::{
    Class, Exclusions, Outcome, PROFILE_NUMBERS, PROFILE_SWITCHES, Profile, Recorder,
    exclusion_paths, exclusion_property, measure, unless_excluded,
};
use crate::clash_groups::{Context, Grouping, Groups, grouping, grouping_parameters};
use crate::clash_severity::{Severities, parse_severity, severities, severity_parameters};
use crate::pairs::{prepare, refuse_declaration, severity};
use crate::selection::{Selection, discipline_of, selector_matches};
use crate::support::table::{Matched, Row, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{Parameters, PropertyRef, Traversal, Unavailable, invalid};
use crate::table_allocation::{KEYS, Key, key_properties, read_keys};

/// The two sides of a pair, as the cell columns name them.
const SIDES: [&str; 2] = ["subject", "counterpart"];

/// Each filled key cell outweighs any number of literal characters: a cell
/// keying more categories is more specific, and literals decide between
/// cells keying as many.
const KEYED: u32 = 1 << 16;
/// The literal characters one pattern counts at most, so that eight
/// patterns stay below [`KEYED`].
const LITERALS: u32 = (KEYED - 1) / 8;

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::optional("severity", ColumnKind::String),
    TableColumn::optional("subject_discipline", ColumnKind::TextPattern),
    TableColumn::optional("subject_selector", ColumnKind::Selector),
    TableColumn::optional("subject_key_1", ColumnKind::TextPattern),
    TableColumn::optional("subject_key_2", ColumnKind::TextPattern),
    TableColumn::optional("subject_key_3", ColumnKind::TextPattern),
    TableColumn::optional("counterpart_discipline", ColumnKind::TextPattern),
    TableColumn::optional("counterpart_selector", ColumnKind::Selector),
    TableColumn::optional("counterpart_key_1", ColumnKind::TextPattern),
    TableColumn::optional("counterpart_key_2", ColumnKind::TextPattern),
    TableColumn::optional("counterpart_key_3", ColumnKind::TextPattern),
    TableColumn::required("penetration_tolerance_metres", ColumnKind::Number),
    TableColumn::optional("clearance_metres", ColumnKind::Number),
    TableColumn::optional("duplicate_tolerance_metres", ColumnKind::Number),
    TableColumn::optional("horizontal_tolerance_metres", ColumnKind::Number),
    TableColumn::optional("vertical_tolerance_metres", ColumnKind::Number),
    TableColumn::optional("volume_tolerance_cubic_metres", ColumnKind::Number),
    TableColumn::optional("report_duplicates", ColumnKind::Boolean),
    TableColumn::optional("report_containment", ColumnKind::Boolean),
    TableColumn::optional("report_intersections", ColumnKind::Boolean),
];

/// Checks pairs of bodies against a clash matrix: per pair of categories, a
/// tolerance profile and a severity.
///
/// The rule selects its subjects and its `counterparts` as `clash` does, and
/// the engine's broad phase proposes candidate pairs within the widest
/// clearance any cell declares. Each row of `cells` keys both sides of a
/// pair (`subject_*` and `counterpart_*`) and gives the tolerance profile
/// `clash` reads from its parameters, plus an optional `severity` and
/// `label`. A side's categories are:
///
/// - `*_discipline`: a pattern over the discipline the object's source plays;
/// - `*_key_1` … `*_key_3`: patterns over the text properties the rule's
///   `key_1` … `key_3` parameters name;
/// - `*_selector`: a selector the object must match, such as an entity type.
///
/// A blank cell accepts any object; an absent property matches no pattern.
/// With `symmetric` (the default) a row covers a pair either way round.
/// Each pair is judged with its single most specific row: the one keying the
/// most categories, then the one with the most literal pattern characters.
/// Rows tied for most specific leave the pair not evaluated, and so does a
/// category that cannot be read when a row testing it could apply. A pair no
/// row covers is ignored, or with `report_unmatched` reported as a finding. A
/// row with every class switched off and no clearance covers its pairs
/// without checking them.
///
/// Exclusions are rule-wide. `exclude_same_system` (on by default) skips
/// pairs reaching a shared system through `system_path`; `exclude_paths`,
/// `exclude_target_property` and `exclude_same_layer` (off by default) are
/// `clash`'s.
pub struct ClashMatrix;

/// One side's categories in a cell.
struct Side<'a> {
    discipline: Option<TextPattern>,
    selector: Option<&'a Selector>,
    keys: [Option<TextPattern>; 3],
}

impl Side<'_> {
    fn read<'a>(
        row: Row<'a>,
        side: &str,
        properties: &[Option<PropertyRef<'_>>; 3],
        case_sensitive: bool,
    ) -> Result<Side<'a>, Unavailable> {
        let mut keys = [None, None, None];
        for ((slot, key), property) in keys.iter_mut().zip(KEYS).zip(properties) {
            let column = format!("{side}_{key}");
            *slot = row.pattern(&column, case_sensitive)?;
            if slot.is_some() && property.is_none() {
                return Err(invalid(format!(
                    "`{column}` is filled, but no `{key}` property is declared"
                )));
            }
        }
        Ok(Side {
            discipline: row.pattern(&format!("{side}_discipline"), case_sensitive)?,
            selector: row.selector(&format!("{side}_selector"))?,
            keys,
        })
    }
}

/// One cell of the matrix.
struct Cell<'a> {
    label: Option<&'a str>,
    sides: [Side<'a>; 2],
    profile: Profile,
    severity: Option<Severity>,
}

struct Declaration<'a> {
    cells: Vec<Cell<'a>>,
    properties: [Option<PropertyRef<'a>>; 3],
    symmetric: bool,
    report_unmatched: bool,
    exclude_paths: Vec<Vec<String>>,
    exclude_target_property: Option<PropertyRef<'a>>,
    exclude_same_layer: bool,
    grouping: Option<Grouping<'a>>,
    severities: Severities,
}

fn row_severity(row: Row<'_>) -> Result<Option<Severity>, Unavailable> {
    row.text("severity")?.map(parse_severity).transpose()
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let properties = key_properties(&parameters)?;
    let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
    let rows = parameters.table("cells")?.unwrap_or_default();
    if rows.is_empty() {
        return Err(invalid("the clash matrix has no cells"));
    }
    let cells = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cell = || -> Result<Cell<'_>, Unavailable> {
                Ok(Cell {
                    label: row.text("label")?,
                    sides: [
                        Side::read(*row, SIDES[0], &properties, case_sensitive)?,
                        Side::read(*row, SIDES[1], &properties, case_sensitive)?,
                    ],
                    profile: Profile::read(&|name| row.number(name), &|name| row.boolean(name))?,
                    severity: row_severity(*row)?,
                })
            };
            cell().map_err(|(reason, message)| (reason, format!("cell {index}: {message}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut exclude_paths = Vec::new();
    if parameters.boolean("exclude_same_system")?.unwrap_or(true) {
        let Some(path) = parameters.string("system_path")? else {
            return Err(invalid(
                "`exclude_same_system` is on unless switched off, and needs `system_path`: \
                 the relationship path from an object to its system",
            ));
        };
        let path: Vec<String> = path.split_whitespace().map(str::to_owned).collect();
        if path.is_empty() {
            return Err(invalid("`system_path` has no steps"));
        }
        Traversal::path(&path)?;
        exclude_paths.push(path);
    }
    exclude_paths.extend(exclusion_paths(&parameters)?);
    Ok(Declaration {
        cells,
        properties,
        symmetric: parameters.boolean("symmetric")?.unwrap_or(true),
        report_unmatched: parameters.boolean("report_unmatched")?.unwrap_or(false),
        exclude_paths,
        exclude_target_property: exclusion_property(&parameters)?,
        exclude_same_layer: parameters.boolean("exclude_same_layer")?.unwrap_or(false),
        grouping: grouping(&parameters)?,
        severities: severities(&parameters)?,
    })
}

/// A filled key cell's weight when its pattern matches.
fn keyed(test: RowTest) -> RowTest {
    match test {
        RowTest::Match(literals) => RowTest::Match(KEYED + literals.min(LITERALS)),
        other => other,
    }
}

/// A row covering a pair either way round: the more specific orientation,
/// undecided when one is and the other does not rule the row out.
fn either(forward: RowTest, backward: RowTest) -> RowTest {
    match (forward, backward) {
        (RowTest::NoMatch, other) | (other, RowTest::NoMatch) => other,
        (RowTest::Match(left), RowTest::Match(right)) => RowTest::Match(left.max(right)),
        _ => RowTest::Undecided,
    }
}

/// An object's categories, read once and only where a cell tests them.
struct Category {
    discipline: Option<Result<String, Unavailable>>,
    keys: [Option<Key>; 3],
    evidence: Vec<Evidence>,
}

/// Reads and caches categories, and tests cells against pairs.
struct Categories<'r, 'd> {
    context: &'r RuleContext<'r>,
    declared: &'d Declaration<'d>,
    /// Which key properties any cell tests.
    keys_used: [bool; 3],
    discipline_used: bool,
    objects: BTreeMap<ObjectId, Category>,
    selections: BTreeMap<(usize, usize, ObjectId), Selection>,
}

impl<'r, 'd> Categories<'r, 'd> {
    fn new(context: &'r RuleContext<'r>, declared: &'d Declaration<'d>) -> Self {
        let sides = || declared.cells.iter().flat_map(|cell| &cell.sides);
        let mut keys_used = [false; 3];
        for side in sides() {
            for (used, key) in keys_used.iter_mut().zip(&side.keys) {
                *used |= key.is_some();
            }
        }
        Self {
            context,
            declared,
            keys_used,
            discipline_used: sides().any(|side| side.discipline.is_some()),
            objects: BTreeMap::new(),
            selections: BTreeMap::new(),
        }
    }

    fn category(&mut self, object: &Object) -> &Category {
        let Self {
            context,
            declared,
            keys_used,
            discipline_used,
            objects,
            ..
        } = self;
        objects.entry(object.id.clone()).or_insert_with(|| {
            let (keys, evidence) = read_keys(context, object, &declared.properties, *keys_used);
            Category {
                discipline: discipline_used.then(|| {
                    discipline_of(context, object, "the clash matrix")
                        .map(|discipline| discipline.as_str().to_owned())
                }),
                keys,
                evidence,
            }
        })
    }

    /// Whether side `side` of cell `cell` accepts `object`; why it cannot
    /// tell is added to `unknown`.
    fn test_side(
        &mut self,
        cell: usize,
        side: usize,
        object: &Object,
        unknown: &mut Vec<Unavailable>,
    ) -> RowTest {
        let (context, declared) = (self.context, self.declared);
        let pattern = &declared.cells[cell].sides[side];
        let mut test = RowTest::Match(0);
        let mut reasons = Vec::new();
        let category = self.category(object);
        if let Some(discipline) = &pattern.discipline {
            test = test.and(match category.discipline.as_ref() {
                Some(Ok(declared)) => keyed(discipline.test(declared)),
                Some(Err(why)) => {
                    reasons.push(why.clone());
                    RowTest::Undecided
                }
                None => unreachable!("disciplines are read when a cell tests one"),
            });
        }
        for (key, value) in pattern.keys.iter().zip(&category.keys) {
            let Some(key) = key else { continue };
            test = test.and(match value {
                Some(Key::Text(value)) => keyed(key.test(value)),
                Some(Key::Absent) => RowTest::NoMatch,
                Some(Key::Unknown(reason, message)) => {
                    reasons.push((reason.clone(), message.clone()));
                    RowTest::Undecided
                }
                None => unreachable!("keys are read when a cell tests them"),
            });
        }
        if test == RowTest::NoMatch {
            return test;
        }
        if let Some(selector) = pattern.selector {
            let selection = self
                .selections
                .entry((cell, side, object.id.clone()))
                .or_insert_with(|| selector_matches(context, selector, object, &mut Vec::new()));
            test = test.and(match selection {
                Selection::Match => RowTest::Match(KEYED),
                Selection::NoMatch => RowTest::NoMatch,
                Selection::NotEvaluated(reason, message) => {
                    reasons.push((reason.clone(), message.clone()));
                    RowTest::Undecided
                }
            });
        }
        if test == RowTest::Undecided {
            unknown.extend(reasons);
        }
        test
    }

    /// Whether cell `cell` covers the pair, and how specifically.
    fn test_pair(
        &mut self,
        cell: usize,
        subject: &Object,
        counterpart: &Object,
        unknown: &mut Vec<Unavailable>,
    ) -> RowTest {
        let symmetric = self.declared.symmetric;
        let mut oriented = |first: &Object, second: &Object, unknown: &mut Vec<Unavailable>| {
            let test = self.test_side(cell, 0, first, unknown);
            if test == RowTest::NoMatch {
                return test;
            }
            test.and(self.test_side(cell, 1, second, unknown))
        };
        let forward = oriented(subject, counterpart, unknown);
        if !symmetric {
            return forward;
        }
        either(forward, oriented(counterpart, subject, unknown))
    }

    /// An object's categories as a reviewer reads them.
    fn describe(&mut self, object: &Object) -> String {
        let properties = self.declared.properties;
        let category = self.category(object);
        let mut shown = Vec::new();
        if let Some(Ok(discipline)) = &category.discipline {
            shown.push(format!("discipline `{discipline}`"));
        }
        for (property, key) in properties.iter().zip(&category.keys) {
            let (Some(property), Some(key)) = (property, key) else {
                continue;
            };
            shown.push(match key {
                Key::Text(text) => format!("{property} `{text}`"),
                Key::Absent => format!("{property} absent"),
                Key::Unknown(..) => format!("{property} unknown"),
            });
        }
        if shown.is_empty() {
            object.id.to_string()
        } else {
            format!("{} ({})", object.id, shown.join(", "))
        }
    }
}

impl Cell<'_> {
    fn name(&self, index: usize) -> String {
        match self.label {
            Some(label) => format!("cell {index} `{label}`"),
            None => format!("cell {index}"),
        }
    }
}

impl RuleCapability for ClashMatrix {
    fn id(&self) -> &'static str {
        "axioval:capability.clash-matrix"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        debug_assert!(
            PROFILE_NUMBERS
                .iter()
                .chain(&PROFILE_SWITCHES)
                .all(|name| COLUMNS.iter().any(|column| column.id == *name)),
            "every profile value is a cell column"
        );
        let mut parameters = vec![
            ParameterDescriptor::required("counterparts", ParameterType::Selector),
            ParameterDescriptor::required("cells", ParameterType::Table(COLUMNS)),
        ];
        for key in KEYS {
            parameters.push(ParameterDescriptor::optional(
                key,
                ParameterType::PropertyReference,
            ));
        }
        parameters.extend([
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("symmetric", ParameterType::Boolean),
            ParameterDescriptor::optional("report_unmatched", ParameterType::Boolean),
            ParameterDescriptor::optional("exclude_same_system", ParameterType::Boolean),
            ParameterDescriptor::optional("system_path", ParameterType::String),
            ParameterDescriptor::optional("exclude_paths", ParameterType::StringList),
            ParameterDescriptor::optional(
                "exclude_target_property",
                ParameterType::PropertyReference,
            ),
            ParameterDescriptor::optional("exclude_same_layer", ParameterType::Boolean),
        ]);
        parameters.extend(grouping_parameters());
        parameters.extend(severity_parameters());
        parameters
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let margin = declared
            .cells
            .iter()
            .map(|cell| cell.profile.margin())
            .fold(0.0, f64::max);
        let prepared = match prepare(context, rule, Some(margin), ProximityProjection::Minimum3d) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let mut exclusions = match Exclusions::new(
            context,
            &declared.exclude_paths,
            declared.exclude_target_property,
            declared.exclude_same_layer,
        ) {
            Ok(exclusions) => exclusions,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let groups = match declared
            .grouping
            .as_ref()
            .map(|grouping| Groups::new(context, grouping))
            .transpose()
        {
            Ok(groups) => groups,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let mut categories = Categories::new(context, &declared);
        let mut recorder = Recorder {
            rule,
            evaluation: CapabilityEvaluation::default(),
            unevaluated: prepared.unevaluated,
            groups,
        };
        let indices: Vec<usize> = (0..declared.cells.len()).collect();

        for pair in &prepared.pairs {
            let (subject, counterpart) = (pair.subject(), pair.counterpart());
            let (Some(subject_object), Some(counterpart_object)) = (
                context.project.object(subject),
                context.project.object(counterpart),
            ) else {
                recorder.unevaluated.push(
                    subject.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    format!("the pair with {counterpart} names an object outside the project"),
                );
                continue;
            };
            let exclusion = exclusions.excluded(subject, counterpart);
            if matches!(exclusion, Ok(Some(_))) {
                continue;
            }
            let mut unknown = Vec::new();
            let matched = match_rows(&indices, RowSelection::MostSpecific, |&cell| {
                categories.test_pair(cell, subject_object, counterpart_object, &mut unknown)
            });
            let (index, cell) = match matched {
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
                        let outcome = Outcome::Finding(
                            Class::Unmatched,
                            format!(
                                "no clash matrix cell covers {} against {}",
                                categories.describe(subject_object),
                                categories.describe(counterpart_object),
                            ),
                        );
                        recorder.record(
                            (subject, counterpart),
                            &Context {
                                measured: None,
                                cell: None,
                            },
                            unless_excluded(outcome, exclusion, counterpart),
                            severity(rule),
                            evidence,
                        );
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
                    recorder.unevaluated.push(subject.clone(), reason, message);
                    continue;
                }
                Matched::Ambiguous(tied) => {
                    let names: Vec<String> = tied
                        .iter()
                        .map(|&index| declared.cells[index].name(index))
                        .collect();
                    recorder.unevaluated.push(
                        subject.clone(),
                        NotEvaluatedReason::InvalidDeclaration,
                        format!(
                            "clash matrix {} cover the pair with {counterpart} equally",
                            names.join(" and ")
                        ),
                    );
                    continue;
                }
            };
            if !cell.profile.checks_anything() {
                continue;
            }
            let measured = match measure(prepared.service, subject, counterpart) {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    recorder.unevaluated.push(subject.clone(), reason, message);
                    continue;
                }
            };
            let (outcome, severity, read) = declared.severities.report(
                context,
                &measured,
                (subject, counterpart),
                cell.profile.judge(&measured, counterpart),
                (cell.severity.clone(), severity(rule)),
            );
            let outcome = match outcome {
                Outcome::Finding(class, message) => Outcome::Finding(
                    class,
                    format!("{message} (clash matrix {})", cell.name(index)),
                ),
                other => other,
            };
            let mut evidence = vec![measured.evidence().clone()];
            evidence.extend(categories.category(subject_object).evidence.iter().cloned());
            evidence.extend(
                categories
                    .category(counterpart_object)
                    .evidence
                    .iter()
                    .cloned(),
            );
            evidence.extend(read);
            evidence.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
            evidence.dedup();
            recorder.record(
                (subject, counterpart),
                &Context {
                    measured: Some(&measured),
                    cell: Some(index),
                },
                unless_excluded(outcome, exclusion, counterpart),
                severity,
                evidence,
            );
        }
        recorder.finish()
    }
}
