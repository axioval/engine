//! The authoring vocabulary of the contract: every expression node kind
//! with its fields and result, the comparison operators and the value
//! types each takes, aggregate functions and sources, slope forms and
//! selector kinds, each with labels and help in English and German.
//!
//! A block editor is generated from these tables, never written by hand:
//! the engine's catalogue (`axioval catalogue`) emits them beside the
//! capabilities and measured values. Every table is in a stable order and
//! a test fails when a kind of the contract is missing from it.

use serde::Serialize;

use crate::measured::{LocalizedText, en_de};

/// The version of the catalogue's JSON form, `major.minor.patch`.
///
/// A consumer reads any catalogue of its own major version: a minor
/// version only adds entries or optional fields, a patch only changes
/// texts. Removing or renaming anything raises the major version.
pub const CATALOGUE_SCHEMA_VERSION: &str = "1.0.0";

/// A value type an expression field takes or a node yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueType {
    Boolean,
    /// A whole plain number.
    Integer,
    /// A number, plain or of a unit (a quantity).
    Number,
    /// A number of unit `rad`.
    Angle,
    Text,
    /// A value of an enumeration; it compares with text.
    Enum,
    Date,
    DateTime,
    /// Any value, `null` included.
    Any,
}

/// The value types every value type list in the catalogue names, labelled.
pub const VALUE_TYPES: &[ValueTypeEntry] = &[
    ValueTypeEntry {
        value_type: ValueType::Boolean,
        label: &en_de("Truth", "Wahrheitswert"),
        help: &en_de(
            "True or false; undecided when a measurement straddles a bound.",
            "Wahr oder falsch; unentschieden, wenn ein Messwert eine Grenze überdeckt.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Integer,
        label: &en_de("Whole number", "Ganzzahl"),
        help: &en_de(
            "A whole plain number, such as a count.",
            "Eine ganze Zahl ohne Einheit, etwa eine Anzahl.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Number,
        label: &en_de("Number", "Zahl"),
        help: &en_de(
            "A plain number or a quantity of a unit, an interval when measured inexactly.",
            "Eine Zahl ohne Einheit oder eine Größe mit Einheit, ein Intervall bei ungenauer Messung.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Angle,
        label: &en_de("Angle", "Winkel"),
        help: &en_de(
            "A plane angle, in radians.",
            "Ein ebener Winkel, im Bogenmaß.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Text,
        label: &en_de("Text", "Text"),
        help: &en_de("A text.", "Ein Text."),
    },
    ValueTypeEntry {
        value_type: ValueType::Enum,
        label: &en_de("Enumeration value", "Aufzählungswert"),
        help: &en_de(
            "One value of an enumeration; it compares with text.",
            "Ein Wert einer Aufzählung; er wird wie Text verglichen.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Date,
        label: &en_de("Date", "Datum"),
        help: &en_de("A calendar day.", "Ein Kalendertag."),
    },
    ValueTypeEntry {
        value_type: ValueType::DateTime,
        label: &en_de("Date and time", "Datum und Uhrzeit"),
        help: &en_de(
            "An instant with its UTC offset.",
            "Ein Zeitpunkt mit seinem Versatz zu UTC.",
        ),
    },
    ValueTypeEntry {
        value_type: ValueType::Any,
        label: &en_de("Any value", "Beliebiger Wert"),
        help: &en_de(
            "Any value; its type is known once the leaves it reads are.",
            "Ein beliebiger Wert; sein Typ steht fest, sobald die gelesenen Werte feststehen.",
        ),
    },
];

/// One value type with its labels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueTypeEntry {
    #[serde(rename = "type")]
    pub value_type: ValueType,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

/// What a field of a node holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FieldKind {
    /// One expression of one of `accepts`.
    Expression { accepts: &'static [ValueType] },
    /// A list of expressions of one of `accepts`, at least `minimum`.
    Expressions {
        accepts: &'static [ValueType],
        minimum: usize,
    },
    /// Expressions by name: a lookup's key per key column.
    ExpressionMap,
    /// `when`/`then` branches of an `if`, at least one.
    Branches,
    /// A literal scalar value, typed by its own `type` field.
    Literal,
    /// A text naming something: a property, parameter, rule or table.
    Name { refers: Refers },
    /// One of `options`.
    Choice { options: &'static [&'static str] },
    /// A comparison operator of [`EXPRESSION_COMPARISONS`].
    ExpressionOperator,
    /// A comparison operator of [`SELECTOR_COMPARISONS`].
    SelectorOperator,
    /// A truth flag.
    Flag,
    /// A nested selector.
    Selector,
    /// A list of nested selectors.
    Selectors,
    /// What an aggregate ranges over, of [`AGGREGATE_SOURCES`].
    AggregateSource,
    /// Relationship steps, each as a `related` selector writes it.
    Path,
    /// A parameter value of the package form (`{"type": …, "value": …}`).
    Value,
    /// An expression of one of `accepts`, nested in a selector.
    SelectorExpression { accepts: &'static [ValueType] },
}

/// What a [`FieldKind::Name`] refers to, so an editor can offer choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Refers {
    PropertySet,
    Property,
    Parameter,
    Derived,
    Table,
    TableColumn,
    Rule,
    ObjectType,
    Classification,
    Class,
    Grouping,
    Discipline,
    SourceField,
    Pattern,
    Label,
}

/// One field of a node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// The field's name in the JSON form.
    pub name: &'static str,
    pub kind: FieldKind,
    pub required: bool,
}

/// How a node's result type follows from its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResultRule {
    /// Always `value`.
    Fixed { value: ValueType },
    /// The type the package or source declares for what it names.
    Declared,
    /// The literal's own type.
    Literal,
    /// `null`.
    Null,
    /// The common type of the fields named (`then` and `else`, or every
    /// operand); `null` joins any type.
    Joined { fields: &'static [&'static str] },
    /// The unit both fields share; whole numbers stay whole.
    SameUnit { fields: &'static [&'static str] },
    /// The product (or, `dividing`, the quotient) of both fields' units;
    /// two whole numbers multiply to a whole number.
    UnitProduct { dividing: bool },
    /// The square root of the field's unit.
    UnitRoot,
    /// A plain number rounds to a whole number; a quantity keeps its unit.
    Rounded,
    /// By the aggregate's function: see [`AGGREGATE_FUNCTIONS`].
    Aggregate,
    /// An angle when converting to an angle, else a plain number.
    Slope,
}

/// The categories expression node kinds are grouped by in an editor.
pub const NODE_CATEGORIES: &[Category] = &[
    Category {
        id: "value",
        label: &en_de("Values", "Werte"),
    },
    Category {
        id: "logic",
        label: &en_de("Logic", "Logik"),
    },
    Category {
        id: "comparison",
        label: &en_de("Comparisons", "Vergleiche"),
    },
    Category {
        id: "conditional",
        label: &en_de("Conditions", "Bedingungen"),
    },
    Category {
        id: "arithmetic",
        label: &en_de("Arithmetic", "Arithmetik"),
    },
    Category {
        id: "trigonometry",
        label: &en_de("Angles and slopes", "Winkel und Neigungen"),
    },
    Category {
        id: "aggregate",
        label: &en_de("Aggregates", "Aggregate"),
    },
    Category {
        id: "ruleOutcome",
        label: &en_de("Other rules", "Andere Regeln"),
    },
    Category {
        id: "text",
        label: &en_de("Text", "Text"),
    },
];

/// One category of node kinds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static [LocalizedText],
}

/// One kind of expression node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeKind {
    /// The `kind` tag in the JSON form.
    pub kind: &'static str,
    /// A [`NODE_CATEGORIES`] id.
    pub category: &'static str,
    /// Every field but `kind`, in documentation order; every node also
    /// takes an optional `label`.
    pub fields: &'static [Field],
    pub result: ResultRule,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

use ValueType as V;

const BOOLEAN: &[ValueType] = &[V::Boolean];
const NUMERIC: &[ValueType] = &[V::Integer, V::Number, V::Angle];
const ANGLE: &[ValueType] = &[V::Angle];
const TEXTUAL: &[ValueType] = &[V::Text, V::Enum];
const ANY: &[ValueType] = &[V::Any];
const ORDERED: &[ValueType] = &[
    V::Integer,
    V::Number,
    V::Angle,
    V::Text,
    V::Enum,
    V::Date,
    V::DateTime,
];
const EQUATABLE: &[ValueType] = &[
    V::Boolean,
    V::Integer,
    V::Number,
    V::Angle,
    V::Text,
    V::Enum,
    V::Date,
    V::DateTime,
];

const fn one(name: &'static str, accepts: &'static [ValueType]) -> Field {
    Field {
        name,
        kind: FieldKind::Expression { accepts },
        required: true,
    }
}

const fn many(name: &'static str, accepts: &'static [ValueType], minimum: usize) -> Field {
    Field {
        name,
        kind: FieldKind::Expressions { accepts, minimum },
        required: true,
    }
}

const fn named(name: &'static str, refers: Refers, required: bool) -> Field {
    Field {
        name,
        kind: FieldKind::Name { refers },
        required,
    }
}

const fn flag(name: &'static str) -> Field {
    Field {
        name,
        kind: FieldKind::Flag,
        required: false,
    }
}

const fn choice(name: &'static str, options: &'static [&'static str], required: bool) -> Field {
    Field {
        name,
        kind: FieldKind::Choice { options },
        required,
    }
}

const OPERAND_NUMBER: &[Field] = &[one("operand", NUMERIC)];
const OPERAND_ANGLE: &[Field] = &[one("operand", ANGLE)];
const OPERAND_TEXT: &[Field] = &[one("operand", TEXTUAL)];
const LEFT_RIGHT_NUMBER: &[Field] = &[one("left", NUMERIC), one("right", NUMERIC)];
const RULE: &[Field] = &[named("rule", Refers::Rule, true)];
const CASE: Field = flag("caseSensitive");
const SLOPE_FORMS: &[&str] = &["ratio", "percent", "angle"];

const NUMBER: ResultRule = ResultRule::Fixed { value: V::Number };
const TRUTH: ResultRule = ResultRule::Fixed { value: V::Boolean };
const TEXT: ResultRule = ResultRule::Fixed { value: V::Text };
const OPERAND_UNIT: ResultRule = ResultRule::SameUnit {
    fields: &["operand"],
};

/// Every expression node kind, in the order of the contract.
pub const EXPRESSION_KINDS: &[NodeKind] = &[
    NodeKind {
        kind: "literal",
        category: "value",
        fields: &[Field {
            name: "value",
            kind: FieldKind::Literal,
            required: true,
        }],
        result: ResultRule::Literal,
        label: &en_de("Constant", "Konstante"),
        help: &en_de(
            "A fixed value: a truth, number, quantity with its unit, text, date or date-time.",
            "Ein fester Wert: Wahrheitswert, Zahl, Größe mit Einheit, Text, Datum oder Zeitpunkt.",
        ),
    },
    NodeKind {
        kind: "null",
        category: "value",
        fields: &[],
        result: ResultRule::Null,
        label: &en_de("No value", "Kein Wert"),
        help: &en_de(
            "The value a source states as absent.",
            "Der Wert, den eine Quelle als nicht vorhanden angibt.",
        ),
    },
    NodeKind {
        kind: "property",
        category: "value",
        fields: &[
            named("propertySet", Refers::PropertySet, false),
            named("property", Refers::Property, true),
            choice("of", &["subject"], false),
        ],
        result: ResultRule::Declared,
        label: &en_de("Property", "Eigenschaft"),
        help: &en_de(
            "A property of the object in scope, or with `of: subject` of the checked object; \
             the set `axioval:measured` reads measured values and `axioval:member` the fields \
             of a measured member.",
            "Eine Eigenschaft des betrachteten Objekts, mit `of: subject` des geprüften Objekts; \
             der Satz `axioval:measured` liest Messwerte und `axioval:member` die Felder eines \
             gemessenen Glieds.",
        ),
    },
    NodeKind {
        kind: "parameter",
        category: "value",
        fields: &[named("name", Refers::Parameter, true)],
        result: ResultRule::Declared,
        label: &en_de("Rule parameter", "Regelparameter"),
        help: &en_de(
            "A parameter of the rule, by name.",
            "Ein Parameter der Regel, nach Namen.",
        ),
    },
    NodeKind {
        kind: "derived",
        category: "value",
        fields: &[named("name", Refers::Derived, true)],
        result: ResultRule::Declared,
        label: &en_de("Derived value", "Abgeleiteter Wert"),
        help: &en_de(
            "A value the ruleset derives, by name.",
            "Ein Wert, den das Regelwerk ableitet, nach Namen.",
        ),
    },
    NodeKind {
        kind: "lookup",
        category: "value",
        fields: &[
            named("table", Refers::Table, true),
            Field {
                name: "keys",
                kind: FieldKind::ExpressionMap,
                required: true,
            },
            named("column", Refers::TableColumn, true),
        ],
        result: ResultRule::Declared,
        label: &en_de("Table lookup", "Tabellenwert"),
        help: &en_de(
            "A cell of the most specific row of a table parameter whose key columns match the keys.",
            "Eine Zelle der spezifischsten Zeile eines Tabellenparameters, deren Schlüsselspalten \
             den Schlüsseln entsprechen.",
        ),
    },
    NodeKind {
        kind: "not",
        category: "logic",
        fields: &[one("operand", BOOLEAN)],
        result: TRUTH,
        label: &en_de("Not", "Nicht"),
        help: &en_de(
            "Whether the operand does not hold.",
            "Ob der Operand nicht gilt.",
        ),
    },
    NodeKind {
        kind: "and",
        category: "logic",
        fields: &[many("operands", BOOLEAN, 1)],
        result: TRUTH,
        label: &en_de("All of", "Alle von"),
        help: &en_de("Whether every operand holds.", "Ob jeder Operand gilt."),
    },
    NodeKind {
        kind: "or",
        category: "logic",
        fields: &[many("operands", BOOLEAN, 1)],
        result: TRUTH,
        label: &en_de("Any of", "Eines von"),
        help: &en_de(
            "Whether at least one operand holds.",
            "Ob mindestens ein Operand gilt.",
        ),
    },
    NodeKind {
        kind: "implies",
        category: "logic",
        fields: &[one("antecedent", BOOLEAN), one("consequent", BOOLEAN)],
        result: TRUTH,
        label: &en_de("If … then", "Wenn … dann"),
        help: &en_de(
            "Whether the consequent holds wherever the antecedent does; true where the \
             antecedent does not hold.",
            "Ob die Folgerung überall dort gilt, wo die Voraussetzung gilt; wahr, wo die \
             Voraussetzung nicht gilt.",
        ),
    },
    NodeKind {
        kind: "xor",
        category: "logic",
        fields: &[one("left", BOOLEAN), one("right", BOOLEAN)],
        result: TRUTH,
        label: &en_de("Exactly one of", "Genau eines von"),
        help: &en_de(
            "Whether exactly one of both holds.",
            "Ob genau einer der beiden Operanden gilt.",
        ),
    },
    NodeKind {
        kind: "compare",
        category: "comparison",
        fields: &[
            Field {
                name: "operator",
                kind: FieldKind::ExpressionOperator,
                required: true,
            },
            one("left", EQUATABLE),
            one("right", EQUATABLE),
            CASE,
        ],
        result: TRUTH,
        label: &en_de("Compare", "Vergleichen"),
        help: &en_de(
            "Whether the left value stands to the right as the operator says; quantities must \
             share a unit's dimension.",
            "Ob der linke Wert zum rechten steht, wie der Operator angibt; Größen müssen dieselbe \
             Dimension haben.",
        ),
    },
    NodeKind {
        kind: "between",
        category: "comparison",
        fields: &[
            one("operand", ORDERED),
            one("low", ORDERED),
            one("high", ORDERED),
            flag("lowInclusive"),
            flag("highInclusive"),
        ],
        result: TRUTH,
        label: &en_de("Between", "Zwischen"),
        help: &en_de(
            "Whether the operand lies between both bounds, each inclusive unless stated otherwise.",
            "Ob der Operand zwischen beiden Grenzen liegt, jede einschließlich, sofern nicht \
             anders angegeben.",
        ),
    },
    NodeKind {
        kind: "oneOf",
        category: "comparison",
        fields: &[
            one("operand", EQUATABLE),
            many("values", EQUATABLE, 1),
            CASE,
        ],
        result: TRUTH,
        label: &en_de("One of", "Einer von"),
        help: &en_de(
            "Whether the operand equals one of the values.",
            "Ob der Operand einem der Werte gleicht.",
        ),
    },
    NodeKind {
        kind: "noneOf",
        category: "comparison",
        fields: &[
            one("operand", EQUATABLE),
            many("values", EQUATABLE, 1),
            CASE,
        ],
        result: TRUTH,
        label: &en_de("None of", "Keiner von"),
        help: &en_de(
            "Whether the operand equals none of the values.",
            "Ob der Operand keinem der Werte gleicht.",
        ),
    },
    NodeKind {
        kind: "isDefined",
        category: "comparison",
        fields: &[one("operand", ANY)],
        result: TRUTH,
        label: &en_de("Has a value", "Hat einen Wert"),
        help: &en_de(
            "Whether the operand has a value, that is, is not `null`.",
            "Ob der Operand einen Wert hat, also nicht `null` ist.",
        ),
    },
    NodeKind {
        kind: "isUndefined",
        category: "comparison",
        fields: &[one("operand", ANY)],
        result: TRUTH,
        label: &en_de("Has no value", "Hat keinen Wert"),
        help: &en_de(
            "Whether the operand is `null`.",
            "Ob der Operand `null` ist.",
        ),
    },
    NodeKind {
        kind: "if",
        category: "conditional",
        fields: &[
            Field {
                name: "branches",
                kind: FieldKind::Branches,
                required: true,
            },
            one("else", ANY),
        ],
        result: ResultRule::Joined {
            fields: &["branches.then", "else"],
        },
        label: &en_de("Choose", "Auswahl"),
        help: &en_de(
            "The `then` of the first branch whose `when` holds, else the `else`.",
            "Das `then` des ersten Zweigs, dessen `when` gilt, sonst das `else`.",
        ),
    },
    NodeKind {
        kind: "coalesce",
        category: "conditional",
        fields: &[many("operands", ANY, 1)],
        result: ResultRule::Joined {
            fields: &["operands"],
        },
        label: &en_de("First value", "Erster Wert"),
        help: &en_de(
            "The first operand that is not `null`.",
            "Der erste Operand, der nicht `null` ist.",
        ),
    },
    NodeKind {
        kind: "add",
        category: "arithmetic",
        fields: LEFT_RIGHT_NUMBER,
        result: ResultRule::SameUnit {
            fields: &["left", "right"],
        },
        label: &en_de("Add", "Addieren"),
        help: &en_de(
            "The sum of two numbers of one unit.",
            "Die Summe zweier Zahlen derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "subtract",
        category: "arithmetic",
        fields: LEFT_RIGHT_NUMBER,
        result: ResultRule::SameUnit {
            fields: &["left", "right"],
        },
        label: &en_de("Subtract", "Subtrahieren"),
        help: &en_de(
            "The difference of two numbers of one unit.",
            "Die Differenz zweier Zahlen derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "multiply",
        category: "arithmetic",
        fields: LEFT_RIGHT_NUMBER,
        result: ResultRule::UnitProduct { dividing: false },
        label: &en_de("Multiply", "Multiplizieren"),
        help: &en_de(
            "The product of two numbers; units multiply.",
            "Das Produkt zweier Zahlen; die Einheiten werden multipliziert.",
        ),
    },
    NodeKind {
        kind: "divide",
        category: "arithmetic",
        fields: LEFT_RIGHT_NUMBER,
        result: ResultRule::UnitProduct { dividing: true },
        label: &en_de("Divide", "Dividieren"),
        help: &en_de(
            "The quotient of two numbers; units divide.",
            "Der Quotient zweier Zahlen; die Einheiten werden geteilt.",
        ),
    },
    NodeKind {
        kind: "negate",
        category: "arithmetic",
        fields: OPERAND_NUMBER,
        result: OPERAND_UNIT,
        label: &en_de("Negate", "Negieren"),
        help: &en_de(
            "The operand with its sign reversed.",
            "Der Operand mit umgekehrtem Vorzeichen.",
        ),
    },
    NodeKind {
        kind: "abs",
        category: "arithmetic",
        fields: OPERAND_NUMBER,
        result: OPERAND_UNIT,
        label: &en_de("Absolute value", "Betrag"),
        help: &en_de(
            "The operand without its sign.",
            "Der Operand ohne Vorzeichen.",
        ),
    },
    NodeKind {
        kind: "min",
        category: "arithmetic",
        fields: &[many("operands", NUMERIC, 1)],
        result: ResultRule::SameUnit {
            fields: &["operands"],
        },
        label: &en_de("Least", "Kleinster Wert"),
        help: &en_de(
            "The least of numbers of one unit.",
            "Der kleinste von Zahlen derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "max",
        category: "arithmetic",
        fields: &[many("operands", NUMERIC, 1)],
        result: ResultRule::SameUnit {
            fields: &["operands"],
        },
        label: &en_de("Greatest", "Größter Wert"),
        help: &en_de(
            "The greatest of numbers of one unit.",
            "Der größte von Zahlen derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "round",
        category: "arithmetic",
        fields: &[one("operand", NUMERIC), one("step", NUMERIC)],
        result: ResultRule::SameUnit {
            fields: &["operand", "step"],
        },
        label: &en_de("Round", "Runden"),
        help: &en_de(
            "The operand rounded to the nearest multiple of the step, of its unit.",
            "Der Operand, gerundet auf das nächste Vielfache der Schrittweite derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "floor",
        category: "arithmetic",
        fields: OPERAND_NUMBER,
        result: ResultRule::Rounded,
        label: &en_de("Round down", "Abrunden"),
        help: &en_de(
            "The greatest whole number not above the operand.",
            "Die größte ganze Zahl, die den Operanden nicht übersteigt.",
        ),
    },
    NodeKind {
        kind: "ceil",
        category: "arithmetic",
        fields: OPERAND_NUMBER,
        result: ResultRule::Rounded,
        label: &en_de("Round up", "Aufrunden"),
        help: &en_de(
            "The least whole number not below the operand.",
            "Die kleinste ganze Zahl, die den Operanden nicht unterschreitet.",
        ),
    },
    NodeKind {
        kind: "sqrt",
        category: "arithmetic",
        fields: OPERAND_NUMBER,
        result: ResultRule::UnitRoot,
        label: &en_de("Square root", "Quadratwurzel"),
        help: &en_de(
            "The square root of the operand; an area's is a length.",
            "Die Quadratwurzel des Operanden; die einer Fläche ist eine Länge.",
        ),
    },
    NodeKind {
        kind: "sin",
        category: "trigonometry",
        fields: OPERAND_ANGLE,
        result: NUMBER,
        label: &en_de("Sine", "Sinus"),
        help: &en_de("The sine of an angle.", "Der Sinus eines Winkels."),
    },
    NodeKind {
        kind: "cos",
        category: "trigonometry",
        fields: OPERAND_ANGLE,
        result: NUMBER,
        label: &en_de("Cosine", "Kosinus"),
        help: &en_de("The cosine of an angle.", "Der Kosinus eines Winkels."),
    },
    NodeKind {
        kind: "tan",
        category: "trigonometry",
        fields: OPERAND_ANGLE,
        result: NUMBER,
        label: &en_de("Tangent", "Tangens"),
        help: &en_de("The tangent of an angle.", "Der Tangens eines Winkels."),
    },
    NodeKind {
        kind: "atan2",
        category: "trigonometry",
        fields: &[one("y", NUMERIC), one("x", NUMERIC)],
        result: ResultRule::Fixed { value: V::Angle },
        label: &en_de("Angle of a vector", "Winkel eines Vektors"),
        help: &en_de(
            "The angle of the vector (x, y), both of one unit.",
            "Der Winkel des Vektors (x, y), beide derselben Einheit.",
        ),
    },
    NodeKind {
        kind: "convertSlope",
        category: "trigonometry",
        fields: &[
            one("operand", NUMERIC),
            choice("from", SLOPE_FORMS, true),
            choice("to", SLOPE_FORMS, true),
        ],
        result: ResultRule::Slope,
        label: &en_de("Convert a slope", "Neigung umrechnen"),
        help: &en_de(
            "A slope restated: 1:12 as a ratio is 8.33 as a percentage and about 4.76° as an angle.",
            "Eine Neigung umgerechnet: 1:12 als Verhältnis sind 8,33 Prozent und etwa 4,76° als \
             Winkel.",
        ),
    },
    NodeKind {
        kind: "aggregate",
        category: "aggregate",
        fields: &[
            choice(
                "function",
                &[
                    "count",
                    "sum",
                    "min",
                    "max",
                    "average",
                    "any",
                    "all",
                    "none",
                    "distinctCount",
                ],
                true,
            ),
            Field {
                name: "over",
                kind: FieldKind::AggregateSource,
                required: true,
            },
            Field {
                name: "where",
                kind: FieldKind::Selector,
                required: false,
            },
            Field {
                name: "value",
                kind: FieldKind::Expression { accepts: ANY },
                required: false,
            },
        ],
        result: ResultRule::Aggregate,
        label: &en_de("Aggregate", "Aggregat"),
        help: &en_de(
            "A count, sum, extreme, average or quantified truth over related objects, a group, \
             a selection or measured members; a member whose membership is undecided widens the \
             result or leaves it not evaluated.",
            "Anzahl, Summe, Extremwert, Mittelwert oder quantifizierte Aussage über verbundene \
             Objekte, eine Gruppe, eine Auswahl oder gemessene Glieder; ein Glied mit \
             unentschiedener Zugehörigkeit weitet das Ergebnis oder lässt es ungeprüft.",
        ),
    },
    NodeKind {
        kind: "ruleOutcome",
        category: "ruleOutcome",
        fields: RULE,
        result: TRUTH,
        label: &en_de("Rule passed", "Regel erfüllt"),
        help: &en_de(
            "Whether another rule of the ruleset passed the object; `null` when it did not \
             select it.",
            "Ob eine andere Regel des Regelwerks das Objekt bestanden hat; `null`, wenn sie es \
             nicht ausgewählt hat.",
        ),
    },
    NodeKind {
        kind: "findingCount",
        category: "ruleOutcome",
        fields: RULE,
        result: ResultRule::Fixed { value: V::Integer },
        label: &en_de("Findings of a rule", "Befunde einer Regel"),
        help: &en_de(
            "How many findings another rule reported about the object.",
            "Wie viele Befunde eine andere Regel zum Objekt gemeldet hat.",
        ),
    },
    NodeKind {
        kind: "deviation",
        category: "ruleOutcome",
        fields: RULE,
        result: NUMBER,
        label: &en_de("Deviation of a rule", "Abweichung einer Regel"),
        help: &en_de(
            "The greatest graded deviation of another rule's findings about the object, relative \
             to its bound; `null` when it reported none graded.",
            "Die größte bewertete Abweichung der Befunde einer anderen Regel zum Objekt, bezogen \
             auf ihre Grenze; `null`, wenn sie keine bewertete gemeldet hat.",
        ),
    },
    NodeKind {
        kind: "concat",
        category: "text",
        fields: &[many("operands", TEXTUAL, 1)],
        result: TEXT,
        label: &en_de("Join texts", "Texte verketten"),
        help: &en_de(
            "The texts joined in order.",
            "Die Texte, der Reihe nach verkettet.",
        ),
    },
    NodeKind {
        kind: "length",
        category: "text",
        fields: OPERAND_TEXT,
        result: ResultRule::Fixed { value: V::Integer },
        label: &en_de("Text length", "Textlänge"),
        help: &en_de(
            "The number of characters of a text.",
            "Die Anzahl der Zeichen eines Textes.",
        ),
    },
    NodeKind {
        kind: "lower",
        category: "text",
        fields: OPERAND_TEXT,
        result: TEXT,
        label: &en_de("Lower case", "Kleinbuchstaben"),
        help: &en_de("The text in lower case.", "Der Text in Kleinbuchstaben."),
    },
    NodeKind {
        kind: "upper",
        category: "text",
        fields: OPERAND_TEXT,
        result: TEXT,
        label: &en_de("Upper case", "Großbuchstaben"),
        help: &en_de("The text in upper case.", "Der Text in Großbuchstaben."),
    },
    NodeKind {
        kind: "trim",
        category: "text",
        fields: OPERAND_TEXT,
        result: TEXT,
        label: &en_de("Trim", "Leerraum entfernen"),
        help: &en_de(
            "The text without surrounding whitespace.",
            "Der Text ohne umgebenden Leerraum.",
        ),
    },
];

/// One comparison operator with the value types it compares.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operator {
    /// The operator's name in the JSON form.
    pub operator: &'static str,
    /// A symbol an editor may show.
    pub symbol: &'static str,
    /// The value types it compares; both sides are of one of them (text
    /// and enumeration values compare with each other, numbers of one
    /// dimension).
    pub accepts: &'static [ValueType],
    /// Whether it takes a value to compare with (every expression
    /// comparison does; a selector's `exists` does not).
    pub takes_value: bool,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

const fn operator(
    operator: &'static str,
    symbol: &'static str,
    accepts: &'static [ValueType],
    label: &'static [LocalizedText],
    help: &'static [LocalizedText],
) -> Operator {
    Operator {
        operator,
        symbol,
        accepts,
        takes_value: true,
        label,
        help,
    }
}

const fn presence(
    name: &'static str,
    symbol: &'static str,
    label: &'static [LocalizedText],
    help: &'static [LocalizedText],
) -> Operator {
    Operator {
        operator: name,
        symbol,
        accepts: ANY,
        takes_value: false,
        label,
        help,
    }
}

macro_rules! comparisons {
    ($($name:literal, $symbol:literal, $accepts:expr, $en:literal, $de:literal, $help_en:literal, $help_de:literal;)*) => {
        &[$(operator($name, $symbol, $accepts, &en_de($en, $de), &en_de($help_en, $help_de))),*]
    };
}

/// The operators of a `compare` expression.
pub const EXPRESSION_COMPARISONS: &[Operator] = comparisons![
    "equals", "=", EQUATABLE, "equals", "gleich",
        "Whether both are equal.", "Ob beide gleich sind.";
    "notEquals", "≠", EQUATABLE, "differs from", "ungleich",
        "Whether both differ.", "Ob beide verschieden sind.";
    "lessThan", "<", ORDERED, "less than", "kleiner als",
        "Whether the left is less; dates compare chronologically.",
        "Ob der linke Wert kleiner ist; Daten werden zeitlich verglichen.";
    "lessThanOrEquals", "≤", ORDERED, "at most", "höchstens",
        "Whether the left is at most the right.", "Ob der linke Wert höchstens den rechten erreicht.";
    "greaterThan", ">", ORDERED, "greater than", "größer als",
        "Whether the left is greater.", "Ob der linke Wert größer ist.";
    "greaterThanOrEquals", "≥", ORDERED, "at least", "mindestens",
        "Whether the left is at least the right.", "Ob der linke Wert mindestens den rechten erreicht.";
    "like", "~", TEXTUAL, "matches the pattern", "entspricht dem Muster",
        "Whether the whole text matches a wildcard pattern (`*`, `?`, `\\` escapes).",
        "Ob der ganze Text einem Platzhaltermuster entspricht (`*`, `?`, `\\` maskiert).";
    "matches", "=~", TEXTUAL, "matches the expression", "entspricht dem Ausdruck",
        "Whether the whole text matches a regular expression.",
        "Ob der ganze Text einem regulären Ausdruck entspricht.";
    "contains", "∋", TEXTUAL, "contains", "enthält",
        "Whether the right text occurs in the left.", "Ob der rechte Text im linken vorkommt.";
];

const SELECTOR_TYPES: &[ValueType] = &[
    V::Boolean,
    V::Integer,
    V::Number,
    V::Text,
    V::Enum,
    V::Date,
    V::DateTime,
];
const SELECTOR_ORDERED: &[ValueType] = &[V::Integer, V::Number, V::Date, V::DateTime];
const TEXT_ONLY: &[ValueType] = &[V::Text];

/// The operators of a `property` or `source` selector.
pub const SELECTOR_COMPARISONS: &[Operator] = &[
    operator(
        "equals",
        "=",
        SELECTOR_TYPES,
        &en_de("equals", "gleich"),
        &en_de(
            "Whether the value equals the given one.",
            "Ob der Wert dem angegebenen gleicht.",
        ),
    ),
    operator(
        "notEquals",
        "≠",
        SELECTOR_TYPES,
        &en_de("differs from", "ungleich"),
        &en_de(
            "Whether the value differs from the given one.",
            "Ob der Wert vom angegebenen abweicht.",
        ),
    ),
    operator(
        "lessThan",
        "<",
        SELECTOR_ORDERED,
        &en_de("less than", "kleiner als"),
        &en_de("Whether the value is less.", "Ob der Wert kleiner ist."),
    ),
    operator(
        "lessThanOrEquals",
        "≤",
        SELECTOR_ORDERED,
        &en_de("at most", "höchstens"),
        &en_de(
            "Whether the value is at most the given one.",
            "Ob der Wert höchstens den angegebenen erreicht.",
        ),
    ),
    operator(
        "greaterThan",
        ">",
        SELECTOR_ORDERED,
        &en_de("greater than", "größer als"),
        &en_de("Whether the value is greater.", "Ob der Wert größer ist."),
    ),
    operator(
        "greaterThanOrEquals",
        "≥",
        SELECTOR_ORDERED,
        &en_de("at least", "mindestens"),
        &en_de(
            "Whether the value is at least the given one.",
            "Ob der Wert mindestens den angegebenen erreicht.",
        ),
    ),
    operator(
        "matches",
        "=~",
        TEXT_ONLY,
        &en_de("matches the expression", "entspricht dem Ausdruck"),
        &en_de(
            "Whether the whole value matches a regular expression.",
            "Ob der ganze Wert einem regulären Ausdruck entspricht.",
        ),
    ),
    operator(
        "like",
        "~",
        TEXT_ONLY,
        &en_de("matches the pattern", "entspricht dem Muster"),
        &en_de(
            "Whether the whole value matches a wildcard pattern (`*`, `?`, `\\` escapes).",
            "Ob der ganze Wert einem Platzhaltermuster entspricht (`*`, `?`, `\\` maskiert).",
        ),
    ),
    operator(
        "contains",
        "∋",
        TEXT_ONLY,
        &en_de("contains", "enthält"),
        &en_de(
            "Whether the given text occurs in the value.",
            "Ob der angegebene Text im Wert vorkommt.",
        ),
    ),
    operator(
        "oneOf",
        "∈",
        TEXT_ONLY,
        &en_de("one of", "einer von"),
        &en_de(
            "Whether the value is one of a list of texts.",
            "Ob der Wert einer aus einer Liste von Texten ist.",
        ),
    ),
    operator(
        "noneOf",
        "∉",
        TEXT_ONLY,
        &en_de("none of", "keiner von"),
        &en_de(
            "Whether the value is none of a list of texts.",
            "Ob der Wert keiner aus einer Liste von Texten ist.",
        ),
    ),
    presence(
        "exists",
        "∃",
        &en_de("is stated", "ist angegeben"),
        &en_de(
            "Whether the source states the property at all.",
            "Ob die Quelle die Eigenschaft überhaupt angibt.",
        ),
    ),
    presence(
        "isEmpty",
        "∅",
        &en_de("is empty", "ist leer"),
        &en_de(
            "Whether the property is stated but empty: null, blank text or an empty list.",
            "Ob die Eigenschaft angegeben, aber leer ist: null, leerer Text oder leere Liste.",
        ),
    ),
    presence(
        "isNotEmpty",
        "≠∅",
        &en_de("has a value", "hat einen Wert"),
        &en_de(
            "Whether the property is stated with a value.",
            "Ob die Eigenschaft mit einem Wert angegeben ist.",
        ),
    ),
];

/// One aggregate function.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateFunctionEntry {
    pub function: &'static str,
    /// Whether it takes a `value`: `required`, `optional` or `none`.
    pub value: &'static str,
    /// The types its `value` may have.
    pub accepts: &'static [ValueType],
    /// Its result: a fixed type, or the value's type (`sameUnit`).
    pub result: ResultRule,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

const VALUE_UNIT: ResultRule = ResultRule::SameUnit { fields: &["value"] };

/// The functions of an `aggregate` expression.
pub const AGGREGATE_FUNCTIONS: &[AggregateFunctionEntry] = &[
    AggregateFunctionEntry {
        function: "count",
        value: "none",
        accepts: &[],
        result: ResultRule::Fixed { value: V::Integer },
        label: &en_de("Number of", "Anzahl"),
        help: &en_de("How many members there are.", "Wie viele Glieder es gibt."),
    },
    AggregateFunctionEntry {
        function: "sum",
        value: "required",
        accepts: NUMERIC,
        result: VALUE_UNIT,
        label: &en_de("Sum of", "Summe"),
        help: &en_de(
            "The sum of the members' values.",
            "Die Summe der Werte der Glieder.",
        ),
    },
    AggregateFunctionEntry {
        function: "min",
        value: "required",
        accepts: NUMERIC,
        result: VALUE_UNIT,
        label: &en_de("Least of", "Kleinster Wert"),
        help: &en_de("The least member value.", "Der kleinste Wert der Glieder."),
    },
    AggregateFunctionEntry {
        function: "max",
        value: "required",
        accepts: NUMERIC,
        result: VALUE_UNIT,
        label: &en_de("Greatest of", "Größter Wert"),
        help: &en_de("The greatest member value.", "Der größte Wert der Glieder."),
    },
    AggregateFunctionEntry {
        function: "average",
        value: "required",
        accepts: NUMERIC,
        result: VALUE_UNIT,
        label: &en_de("Average of", "Mittelwert"),
        help: &en_de(
            "The members' mean value; whole numbers average to a number.",
            "Der Mittelwert der Glieder; ganze Zahlen ergeben eine Zahl.",
        ),
    },
    AggregateFunctionEntry {
        function: "any",
        value: "required",
        accepts: BOOLEAN,
        result: TRUTH,
        label: &en_de("Some member", "Ein Glied"),
        help: &en_de(
            "Whether the value holds for some member.",
            "Ob der Wert für irgendein Glied gilt.",
        ),
    },
    AggregateFunctionEntry {
        function: "all",
        value: "required",
        accepts: BOOLEAN,
        result: TRUTH,
        label: &en_de("Every member", "Jedes Glied"),
        help: &en_de(
            "Whether the value holds for every member, and there is a member.",
            "Ob der Wert für jedes Glied gilt und es mindestens ein Glied gibt.",
        ),
    },
    AggregateFunctionEntry {
        function: "none",
        value: "required",
        accepts: BOOLEAN,
        result: TRUTH,
        label: &en_de("No member", "Kein Glied"),
        help: &en_de(
            "Whether the value holds for no member; true without members.",
            "Ob der Wert für kein Glied gilt; wahr ohne Glieder.",
        ),
    },
    AggregateFunctionEntry {
        function: "distinctCount",
        value: "optional",
        accepts: EQUATABLE,
        result: ResultRule::Fixed { value: V::Integer },
        label: &en_de("Number of distinct", "Anzahl verschiedener"),
        help: &en_de(
            "How many distinct values the members have.",
            "Wie viele verschiedene Werte die Glieder haben.",
        ),
    },
];

/// One kind of what an aggregate ranges over.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceKind {
    pub kind: &'static str,
    pub fields: &'static [Field],
    /// Whether the aggregate may filter it with `where`.
    pub takes_where: bool,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

/// What an `aggregate` ranges over (its `over`).
pub const AGGREGATE_SOURCES: &[SourceKind] = &[
    SourceKind {
        kind: "path",
        fields: &[Field {
            name: "path",
            kind: FieldKind::Path,
            required: true,
        }],
        takes_where: true,
        label: &en_de("Related objects", "Verbundene Objekte"),
        help: &en_de(
            "The objects a relationship path reaches from the object in scope.",
            "Die Objekte, die ein Beziehungspfad vom betrachteten Objekt aus erreicht.",
        ),
    },
    SourceKind {
        kind: "group",
        fields: &[named("grouping", Refers::Grouping, true)],
        takes_where: true,
        label: &en_de("Group members", "Gruppenmitglieder"),
        help: &en_de(
            "The members of the derived group the object in scope is or belongs to.",
            "Die Mitglieder der abgeleiteten Gruppe, die das betrachtete Objekt ist oder zu der es gehört.",
        ),
    },
    SourceKind {
        kind: "selector",
        fields: &[Field {
            name: "selector",
            kind: FieldKind::Selector,
            required: true,
        }],
        takes_where: true,
        label: &en_de("Selected objects", "Ausgewählte Objekte"),
        help: &en_de(
            "Every object of the project a selector selects.",
            "Jedes Objekt des Projekts, das ein Selektor auswählt.",
        ),
    },
    SourceKind {
        kind: "measured",
        fields: &[Field {
            name: "name",
            kind: FieldKind::Name {
                refers: Refers::Label,
            },
            required: true,
        }],
        takes_where: false,
        label: &en_de("Measured members", "Gemessene Glieder"),
        help: &en_de(
            "The members built-in code measures of the object in scope, by a measured member \
             list; its value reads their fields in `axioval:member`.",
            "Die Glieder, die eingebauter Code am betrachteten Objekt misst, nach einer Liste \
             gemessener Glieder; der Wert liest ihre Felder in `axioval:member`.",
        ),
    },
];

/// One kind of selector.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorKind {
    pub kind: &'static str,
    pub fields: &'static [Field],
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

const OPERATOR: Field = Field {
    name: "operator",
    kind: FieldKind::SelectorOperator,
    required: true,
};
const VALUE: Field = Field {
    name: "value",
    kind: FieldKind::Value,
    required: false,
};
const QUANTIFIER: Field = choice("quantifier", &["any", "all"], false);

/// Every selector kind a package may write.
pub const SELECTOR_KINDS: &[SelectorKind] = &[
    SelectorKind {
        kind: "all",
        fields: &[],
        label: &en_de("Every object", "Alle Objekte"),
        help: &en_de("Every model object.", "Jedes Modellobjekt."),
    },
    SelectorKind {
        kind: "entityType",
        fields: &[
            named("objectType", Refers::ObjectType, true),
            flag("includeSubtypes"),
        ],
        label: &en_de("Object type", "Objekttyp"),
        help: &en_de(
            "Objects of a type, its subtypes included unless stated otherwise.",
            "Objekte eines Typs, einschließlich seiner Untertypen, sofern nicht anders angegeben.",
        ),
    },
    SelectorKind {
        kind: "property",
        fields: &[
            named("propertySet", Refers::PropertySet, false),
            named("property", Refers::Property, true),
            OPERATOR,
            VALUE,
            flag("caseSensitive"),
            flag("trim"),
            QUANTIFIER,
            choice("precision", &["day"], false),
        ],
        label: &en_de("Property value", "Eigenschaftswert"),
        help: &en_de(
            "Objects whose property compares with a value as the operator says.",
            "Objekte, deren Eigenschaft sich zum Wert verhält, wie der Operator angibt.",
        ),
    },
    SelectorKind {
        kind: "propertyPattern",
        fields: &[
            named("propertySetPattern", Refers::Pattern, false),
            named("propertyPattern", Refers::Pattern, true),
            OPERATOR,
            VALUE,
            choice("matched", &["any", "all"], false),
            flag("caseSensitive"),
            flag("trim"),
            QUANTIFIER,
            choice("precision", &["day"], false),
        ],
        label: &en_de("Property by pattern", "Eigenschaft nach Muster"),
        help: &en_de(
            "Objects by the properties whose set and name match patterns, as IDS names them.",
            "Objekte nach den Eigenschaften, deren Satz und Name Mustern entsprechen, wie IDS sie \
             benennt.",
        ),
    },
    SelectorKind {
        kind: "classification",
        fields: &[
            named("system", Refers::Classification, true),
            named("code", Refers::Class, false),
            named("codePattern", Refers::Pattern, false),
            flag("includeDescendants"),
        ],
        label: &en_de("Classification", "Klassifikation"),
        help: &en_de(
            "Objects carrying a classification code of a system, as their source states it.",
            "Objekte mit einem Klassifikationscode eines Systems, wie ihre Quelle ihn angibt.",
        ),
    },
    SelectorKind {
        kind: "derivedClass",
        fields: &[
            named("classification", Refers::Classification, true),
            named("class", Refers::Class, true),
            flag("includeDescendants"),
        ],
        label: &en_de("Derived class", "Abgeleitete Klasse"),
        help: &en_de(
            "Objects a classification of the ruleset assigns a class.",
            "Objekte, denen eine Klassifikation des Regelwerks eine Klasse zuweist.",
        ),
    },
    SelectorKind {
        kind: "derivedGroup",
        fields: &[named("grouping", Refers::Grouping, true)],
        label: &en_de("Derived groups", "Abgeleitete Gruppen"),
        help: &en_de(
            "The groups a grouping of the ruleset derives, one object per group.",
            "Die Gruppen, die eine Gruppierung des Regelwerks bildet, ein Objekt je Gruppe.",
        ),
    },
    SelectorKind {
        kind: "allOf",
        fields: &[Field {
            name: "operands",
            kind: FieldKind::Selectors,
            required: true,
        }],
        label: &en_de("All of", "Alle von"),
        help: &en_de(
            "Objects every operand selects.",
            "Objekte, die jeder Operand auswählt.",
        ),
    },
    SelectorKind {
        kind: "anyOf",
        fields: &[Field {
            name: "operands",
            kind: FieldKind::Selectors,
            required: true,
        }],
        label: &en_de("Any of", "Eines von"),
        help: &en_de(
            "Objects at least one operand selects.",
            "Objekte, die mindestens ein Operand auswählt.",
        ),
    },
    SelectorKind {
        kind: "not",
        fields: &[Field {
            name: "operand",
            kind: FieldKind::Selector,
            required: true,
        }],
        label: &en_de("Not", "Nicht"),
        help: &en_de(
            "Objects the operand does not select.",
            "Objekte, die der Operand nicht auswählt.",
        ),
    },
    SelectorKind {
        kind: "related",
        fields: &[
            Field {
                name: "path",
                kind: FieldKind::Path,
                required: true,
            },
            choice("quantifier", &["any", "all", "none"], false),
            Field {
                name: "selector",
                kind: FieldKind::Selector,
                required: true,
            },
        ],
        label: &en_de("Related objects", "Verbundene Objekte"),
        help: &en_de(
            "Objects by the objects a relationship path reaches from them.",
            "Objekte nach den Objekten, die ein Beziehungspfad von ihnen aus erreicht.",
        ),
    },
    SelectorKind {
        kind: "discipline",
        fields: &[named("value", Refers::Discipline, true)],
        label: &en_de("Discipline", "Fachdisziplin"),
        help: &en_de(
            "Objects of the sources declared to play a discipline, such as structure.",
            "Objekte der Quellen, die einer Fachdisziplin zugeordnet sind, etwa Tragwerk.",
        ),
    },
    SelectorKind {
        kind: "source",
        fields: &[
            named("field", Refers::SourceField, true),
            OPERATOR,
            VALUE,
            flag("caseSensitive"),
            flag("trim"),
            QUANTIFIER,
        ],
        label: &en_de("Source metadata", "Quellmetadaten"),
        help: &en_de(
            "Objects of the sources whose metadata field compares as the operator says.",
            "Objekte der Quellen, deren Metadatenfeld sich verhält, wie der Operator angibt.",
        ),
    },
    SelectorKind {
        kind: "ruleOutcome",
        fields: &[
            named("rule", Refers::Rule, true),
            choice("outcome", &["passed", "failed"], true),
        ],
        label: &en_de("Judged by a rule", "Von einer Regel beurteilt"),
        help: &en_de(
            "Objects another rule of the ruleset passed or reported a finding about.",
            "Objekte, die eine andere Regel des Regelwerks bestanden oder beanstandet hat.",
        ),
    },
    SelectorKind {
        kind: "expression",
        fields: &[Field {
            name: "expression",
            kind: FieldKind::SelectorExpression { accepts: BOOLEAN },
            required: true,
        }],
        label: &en_de("Condition", "Bedingung"),
        help: &en_de(
            "Objects for which a truth expression holds.",
            "Objekte, für die ein Wahrheitsausdruck gilt.",
        ),
    },
];

/// One way a slope is stated.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SlopeFormEntry {
    pub form: &'static str,
    pub label: &'static [LocalizedText],
}

/// How a `convertSlope` states a slope.
pub const SLOPE_FORM_ENTRIES: &[SlopeFormEntry] = &[
    SlopeFormEntry {
        form: "ratio",
        label: &en_de("Rise over run", "Steigungsverhältnis"),
    },
    SlopeFormEntry {
        form: "percent",
        label: &en_de("Percent", "Prozent"),
    },
    SlopeFormEntry {
        form: "angle",
        label: &en_de("Angle", "Winkel"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Expression, Selector};

    /// Names every variant of `$enum` by an exhaustive match, so a new
    /// variant fails to compile until it is named here, and returns the
    /// names.
    macro_rules! covered {
        ($enum:ty; $($pattern:pat => $name:literal),* $(,)?) => {{
            #[allow(dead_code, unreachable_patterns)]
            fn name(value: &$enum) -> &'static str {
                match value {
                    $($pattern => $name),*
                }
            }
            vec![$($name),*]
        }};
    }

    fn texts_complete(texts: &[LocalizedText], what: &str) {
        let languages: Vec<&str> = texts.iter().map(|text| text.language).collect();
        assert_eq!(languages, ["en", "de"], "{what}");
        assert!(
            texts.iter().all(|text| !text.text.trim().is_empty()),
            "{what}"
        );
    }

    #[test]
    fn every_expression_kind_is_catalogued() {
        use Expression as E;
        let kinds = covered![Expression;
            E::Literal { .. } => "literal", E::Null { .. } => "null",
            E::Property { .. } => "property", E::Parameter { .. } => "parameter",
            E::Derived { .. } => "derived", E::Lookup { .. } => "lookup",
            E::Not { .. } => "not", E::And { .. } => "and", E::Or { .. } => "or",
            E::Implies { .. } => "implies", E::Xor { .. } => "xor",
            E::Compare { .. } => "compare", E::Between { .. } => "between",
            E::OneOf { .. } => "oneOf", E::NoneOf { .. } => "noneOf",
            E::IsDefined { .. } => "isDefined", E::IsUndefined { .. } => "isUndefined",
            E::If { .. } => "if", E::Coalesce { .. } => "coalesce",
            E::Add { .. } => "add", E::Subtract { .. } => "subtract",
            E::Multiply { .. } => "multiply", E::Divide { .. } => "divide",
            E::Negate { .. } => "negate", E::Abs { .. } => "abs",
            E::Min { .. } => "min", E::Max { .. } => "max", E::Round { .. } => "round",
            E::Floor { .. } => "floor", E::Ceil { .. } => "ceil", E::Sqrt { .. } => "sqrt",
            E::Sin { .. } => "sin", E::Cos { .. } => "cos", E::Tan { .. } => "tan",
            E::Atan2 { .. } => "atan2", E::ConvertSlope { .. } => "convertSlope",
            E::Aggregate { .. } => "aggregate", E::RuleOutcome { .. } => "ruleOutcome",
            E::FindingCount { .. } => "findingCount", E::Deviation { .. } => "deviation",
            E::Concat { .. } => "concat", E::Length { .. } => "length",
            E::Lower { .. } => "lower", E::Upper { .. } => "upper", E::Trim { .. } => "trim",
        ];
        let catalogued: Vec<&str> = EXPRESSION_KINDS.iter().map(|kind| kind.kind).collect();
        assert_eq!(catalogued, kinds);
        for kind in EXPRESSION_KINDS {
            // The tag is the contract's: an unknown one names the variant.
            let error =
                serde_json::from_value::<Expression>(serde_json::json!({"kind": kind.kind}))
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default();
            assert!(!error.contains("unknown variant"), "{}: {error}", kind.kind);
            assert!(
                NODE_CATEGORIES
                    .iter()
                    .any(|category| category.id == kind.category)
            );
            texts_complete(kind.label, kind.kind);
            texts_complete(kind.help, kind.kind);
        }
    }

    #[test]
    fn every_selector_kind_is_catalogued() {
        use Selector as S;
        let kinds = covered![Selector;
            S::All => "all", S::EntityType { .. } => "entityType",
            S::Property { .. } => "property", S::PropertyPattern { .. } => "propertyPattern",
            S::Classification { .. } => "classification", S::DerivedClass { .. } => "derivedClass",
            S::DerivedGroup { .. } => "derivedGroup", S::AllOf { .. } => "allOf",
            S::AnyOf { .. } => "anyOf", S::Not { .. } => "not", S::Related { .. } => "related",
            S::Discipline { .. } => "discipline", S::Source { .. } => "source",
            S::RuleOutcome { .. } => "ruleOutcome", S::Expression { .. } => "expression",
            // Engine-internal; never written in a package.
            S::Objects { .. } => "",
        ];
        let written: Vec<&str> = kinds.into_iter().filter(|kind| !kind.is_empty()).collect();
        let catalogued: Vec<&str> = SELECTOR_KINDS.iter().map(|kind| kind.kind).collect();
        assert_eq!(catalogued, written);
        for kind in SELECTOR_KINDS {
            let error = serde_json::from_value::<Selector>(serde_json::json!({"kind": kind.kind}))
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            assert!(!error.contains("unknown variant"), "{}: {error}", kind.kind);
            texts_complete(kind.label, kind.kind);
            texts_complete(kind.help, kind.kind);
        }
    }

    #[test]
    fn every_operator_function_and_source_is_catalogued() {
        use crate::contract::{
            AggregateFunction as F, AggregateSource as A, ComparisonOperator as O,
            ExpressionComparison as C, SlopeForm,
        };
        let comparisons = covered![C;
            C::Equals => "equals", C::NotEquals => "notEquals", C::LessThan => "lessThan",
            C::LessThanOrEquals => "lessThanOrEquals", C::GreaterThan => "greaterThan",
            C::GreaterThanOrEquals => "greaterThanOrEquals", C::Like => "like",
            C::Matches => "matches", C::Contains => "contains",
        ];
        let names = |operators: &[Operator]| -> Vec<&'static str> {
            operators.iter().map(|operator| operator.operator).collect()
        };
        assert_eq!(names(EXPRESSION_COMPARISONS), comparisons);
        let operators = covered![O;
            O::Equals => "equals", O::NotEquals => "notEquals", O::LessThan => "lessThan",
            O::LessThanOrEquals => "lessThanOrEquals", O::GreaterThan => "greaterThan",
            O::GreaterThanOrEquals => "greaterThanOrEquals", O::Matches => "matches",
            O::Like => "like", O::Contains => "contains", O::OneOf => "oneOf",
            O::NoneOf => "noneOf", O::Exists => "exists", O::IsEmpty => "isEmpty",
            O::IsNotEmpty => "isNotEmpty",
        ];
        assert_eq!(names(SELECTOR_COMPARISONS), operators);
        let functions = covered![F;
            F::Count => "count", F::Sum => "sum", F::Min => "min", F::Max => "max",
            F::Average => "average", F::Any => "any", F::All => "all", F::None => "none",
            F::DistinctCount => "distinctCount",
        ];
        let catalogued: Vec<&str> = AGGREGATE_FUNCTIONS
            .iter()
            .map(|entry| entry.function)
            .collect();
        assert_eq!(catalogued, functions);
        let sources = covered![A;
            A::Path { .. } => "path", A::Group { .. } => "group",
            A::Selector { .. } => "selector", A::Measured { .. } => "measured",
        ];
        let catalogued: Vec<&str> = AGGREGATE_SOURCES.iter().map(|entry| entry.kind).collect();
        assert_eq!(catalogued, sources);
        let forms = covered![SlopeForm;
            SlopeForm::Ratio => "ratio", SlopeForm::Percent => "percent", SlopeForm::Angle => "angle",
        ];
        let catalogued: Vec<&str> = SLOPE_FORM_ENTRIES.iter().map(|entry| entry.form).collect();
        assert_eq!(catalogued, forms);
        // Each spelling is the contract's.
        for name in comparisons {
            serde_json::from_value::<C>(serde_json::json!(name)).unwrap();
        }
        for name in operators {
            serde_json::from_value::<O>(serde_json::json!(name)).unwrap();
        }
        for name in functions {
            serde_json::from_value::<F>(serde_json::json!(name)).unwrap();
        }
        for name in forms {
            serde_json::from_value::<SlopeForm>(serde_json::json!(name)).unwrap();
        }
        for operator in EXPRESSION_COMPARISONS.iter().chain(SELECTOR_COMPARISONS) {
            texts_complete(operator.label, operator.operator);
            texts_complete(operator.help, operator.operator);
        }
        for entry in AGGREGATE_FUNCTIONS {
            texts_complete(entry.label, entry.function);
            texts_complete(entry.help, entry.function);
        }
        for entry in AGGREGATE_SOURCES {
            texts_complete(entry.label, entry.kind);
            texts_complete(entry.help, entry.kind);
        }
        for entry in VALUE_TYPES {
            texts_complete(entry.label, "value type");
            texts_complete(entry.help, "value type");
        }
        for category in NODE_CATEGORIES {
            texts_complete(category.label, category.id);
        }
    }

    #[test]
    fn aggregate_function_options_match_the_table() {
        let Some(FieldKind::Choice { options }) = EXPRESSION_KINDS
            .iter()
            .find(|kind| kind.kind == "aggregate")
            .and_then(|kind| kind.fields.first())
            .map(|field| field.kind)
        else {
            panic!("aggregate has a function choice");
        };
        let functions: Vec<&str> = AGGREGATE_FUNCTIONS
            .iter()
            .map(|entry| entry.function)
            .collect();
        assert_eq!(options.to_vec(), functions);
    }
}
