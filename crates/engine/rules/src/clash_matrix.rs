//! A clash matrix: tolerances and severities per pair of categories.
//!
//! Each candidate pair is judged with the tolerance profile of the one matrix
//! cell that covers it most specifically. A cell keys each side of the pair
//! by category: the discipline the object's source plays, key properties and
//! a selector, each optional. The pair judgement itself (classes, interval
//! handling, exclusions, measurement) is `clash`'s, shared through
//! [`crate::clash`], so a cell means exactly what a `clash` rule with the
//! same values means.
//!
//! It runs as a template: the measured list `clash_matrix_pairs` picks each
//! pair's cell (a search over the cells' categories) and measures the pair
//! beside that cell's tolerances, and the template judges it as `clash`'s
//! does, by the cell's severity.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Object, ObjectId, Severity};

use crate::clash::{
    PROFILE_NUMBERS, PROFILE_SWITCHES, Profile, exclusion_paths, exclusion_property,
};
use crate::clash_cases::{Cases, case_parameter, cases};
use crate::clash_groups::{Grouping, grouping, grouping_parameters};
use crate::clash_severity::{Severities, parse_severity, severities, severity_parameters};
use crate::selection::{Selection, discipline_of, selector_matches};
use crate::support::table::{Row, RowTest, TextPattern};
use crate::support::{Parameters, PropertyRef, Traversal, Unavailable, invalid};
use crate::table_allocation::{KEY_COUNT, KEYS, Key, KeyProperties, key_properties, read_keys};

/// The declaration the capability refused, in its order and words: what
/// the template's `Check::Arguments` refuses once per rule. `stated` holds
/// the rule's parameters the list names, by their own names.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    declaration(&crate::clash::synthesised(stated.clone())).map(|_| ())
}

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
pub(crate) struct Side<'a> {
    discipline: Option<TextPattern>,
    selector: Option<&'a Selector>,
    keys: [Option<TextPattern>; KEY_COUNT],
}

impl Side<'_> {
    fn read<'a>(
        row: Row<'a>,
        side: &str,
        properties: &KeyProperties<'_>,
        case_sensitive: bool,
    ) -> Result<Side<'a>, Unavailable> {
        let mut keys = [None, None, None, None];
        for ((slot, key), property) in keys.iter_mut().zip(&KEYS[..3]).zip(properties) {
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
pub(crate) struct Cell<'a> {
    label: Option<&'a str>,
    sides: [Side<'a>; 2],
    pub(crate) profile: Profile,
    pub(crate) severity: Option<Severity>,
}

/// A `clash-matrix` rule's declaration, read as the capability reads it.
pub(crate) struct Declaration<'a> {
    pub(crate) cells: Vec<Cell<'a>>,
    properties: KeyProperties<'a>,
    symmetric: bool,
    pub(crate) report_unmatched: bool,
    pub(crate) exclude_paths: Vec<Vec<String>>,
    pub(crate) exclude_target_property: Option<PropertyRef<'a>>,
    pub(crate) exclude_same_layer: bool,
    pub(crate) grouping: Option<Grouping<'a>>,
    pub(crate) severities: Severities,
    pub(crate) cases: Cases<'a>,
}

fn row_severity(row: Row<'_>) -> Result<Option<Severity>, Unavailable> {
    row.text("severity")?.map(parse_severity).transpose()
}

/// Reads a `clash-matrix` rule's declaration, refusing it in the
/// capability's order and words.
pub(crate) fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
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
        cases: cases(&parameters)?,
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
pub(crate) struct Category {
    discipline: Option<Result<String, Unavailable>>,
    keys: [Option<Key>; KEY_COUNT],
    pub(crate) evidence: Vec<Evidence>,
}

/// Reads and caches categories, and tests cells against pairs.
pub(crate) struct Categories<'r, 'd> {
    context: &'r RuleContext<'r>,
    declared: &'d Declaration<'d>,
    /// Which key properties any cell tests.
    keys_used: [bool; KEY_COUNT],
    discipline_used: bool,
    objects: BTreeMap<ObjectId, Category>,
    selections: BTreeMap<(usize, usize, ObjectId), Selection>,
}

impl<'r, 'd> Categories<'r, 'd> {
    pub(crate) fn new(context: &'r RuleContext<'r>, declared: &'d Declaration<'d>) -> Self {
        let sides = || declared.cells.iter().flat_map(|cell| &cell.sides);
        let mut keys_used = [false; KEY_COUNT];
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

    pub(crate) fn category(&mut self, object: &Object) -> &Category {
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
    pub(crate) fn test_pair(
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
    pub(crate) fn describe(&mut self, object: &Object) -> String {
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
    pub(crate) fn name(&self, index: usize) -> String {
        match self.label {
            Some(label) => format!("cell {index} `{label}`"),
            None => format!("cell {index}"),
        }
    }
}

impl RuleCapability for ClashMatrix {
    fn id(&self) -> &'static str {
        crate::clash::template::CLASH_MATRIX
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

static TEMPLATE: LazyLock<Template> = LazyLock::new(crate::clash::template::clash_matrix);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

/// The descriptor `clash-matrix` keeps.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
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
    // The matrix keys three properties; `key_4` is not among them.
    for key in KEYS.into_iter().take(3) {
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
        ParameterDescriptor::optional("exclude_target_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("exclude_same_layer", ParameterType::Boolean),
    ]);
    parameters.extend(grouping_parameters());
    parameters.extend(severity_parameters());
    parameters.push(case_parameter());
    parameters
}
