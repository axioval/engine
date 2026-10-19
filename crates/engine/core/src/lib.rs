//! Trusted capability compilation and deterministic runtime.
#![forbid(unsafe_code)]
#![allow(missing_docs, clippy::missing_errors_doc)]

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract as schema;
use axioval_ir::{
    Finding, Location, NotEvaluated, ObjectId, Project, Report, ReportTable, RuleId, RuleSummary,
    Scope, SourceId,
};
use thiserror::Error;

mod session;

pub mod comparison;

pub mod decimal;

/// Expressions: units, intervals, evaluation and type checking.
pub mod expression;

/// Writes names as the MCS binder lists them: `['a', 'b']`.
fn quoted_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("'{name}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Errors while compiling untrusted declarations into a trusted execution plan.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    /// A package declares a schema version this compiler does not implement.
    #[error(
        "unsupported schema version `{version}` for {package_kind} `{package_id}`; supported: {supported}"
    )]
    UnsupportedSchemaVersion {
        package_kind: &'static str,
        package_id: String,
        version: String,
        supported: &'static str,
    },
    /// Multiple supplied definition packages declared the same package identity.
    #[error("duplicate definition package `{0}`")]
    DuplicateDefinitionPackage(String),
    /// A capability was not registered by the host.
    #[error("unknown capability `{0}`")]
    UnknownCapability(String),
    /// Two trusted implementations claimed an ID.
    #[error("duplicate capability `{0}`")]
    DuplicateCapability(String),
    /// A measured-value provider claimed a name that is not registered,
    /// that the engine measures itself, or that another provider measures.
    #[error("measured value `{0}` cannot be provided: {1}")]
    InvalidProvider(String, String),
    /// A package supplied a non-declared parameter.
    #[error("capability `{capability}` does not declare parameter `{parameter}`")]
    UnknownParameter {
        capability: String,
        parameter: String,
    },
    /// A required parameter was absent.
    #[error("capability `{capability}` requires parameter `{parameter}`")]
    MissingParameter {
        capability: String,
        parameter: String,
    },
    /// A rule instance states parameters its definition fixes. A fixed
    /// parameter takes the definition's default; changing it means forking
    /// the definition. `parameters` is sorted.
    #[error(
        "rule `{rule}`: parameters {} are fixed by definition '{definition}'; instances may not \
         state them, and changing them means forking the definition",
        quoted_list(.parameters)
    )]
    FixedParameter {
        rule: String,
        definition: String,
        parameters: Vec<String>,
    },
    /// A definition fixes a parameter without giving it a default.
    #[error(
        "definition `{definition}` parameter `{parameter}`: a fixed parameter requires a defaultValue"
    )]
    FixedWithoutDefault {
        definition: String,
        parameter: String,
    },
    /// A binding type did not conform to its descriptor.
    #[error("capability `{capability}` parameter `{parameter}` has invalid type")]
    InvalidParameterType {
        capability: String,
        parameter: String,
    },
    /// A row of a table-valued binding does not fit the declared columns.
    #[error("capability `{capability}` parameter `{parameter}` row {row}: {detail}")]
    InvalidTableRow {
        capability: String,
        parameter: String,
        /// Zero-based row index.
        row: usize,
        detail: String,
    },
    /// A table-valued binding names a data file that was not loaded, or
    /// whose declared columns do not fit the parameter's table.
    #[error("capability `{capability}` parameter `{parameter}` table file `{path}`: {detail}")]
    InvalidTableFile {
        capability: String,
        parameter: String,
        path: String,
        detail: String,
    },
    /// A rule binds a parameter more than once.
    #[error("rule has duplicate parameter binding `{0}`")]
    DuplicateBinding(String),
    /// A rule ID violates the engine identity contract.
    #[error("invalid rule id `{0}`")]
    InvalidRuleId(String),
    /// A rule references no loaded definition.
    #[error("unknown rule definition `{0}`")]
    UnknownDefinition(String),
    /// A ruleset references a definition package that was not supplied.
    #[error("missing definition package `{0}`")]
    MissingDefinitionPackage(String),
    /// A trusted capability descriptor conflicts with its portable definition.
    #[error("definition `{definition}` conflicts with capability `{capability}`: {detail}")]
    CapabilityContract {
        definition: String,
        capability: String,
        detail: String,
    },
    /// Rule IDs must be unique throughout the recursive folder tree.
    #[error("duplicate rule id `{0}`")]
    DuplicateRule(String),
    /// Two rulesets compiled together share a package ID.
    #[error("duplicate ruleset package `{0}`")]
    DuplicateRuleSet(String),
    /// No ruleset was given to compile.
    #[error("no ruleset to compile")]
    NoRuleSet,
    /// Two loaded definition packages declare the same concept identity.
    #[error("duplicate concept `{0}` across definition packages")]
    DuplicateConcept(String),
    /// A rule names a concept no loaded definition package declares.
    #[error("rule `{rule}` references unknown {kind} concept `{concept}`")]
    UnknownConcept {
        rule: String,
        kind: String,
        concept: String,
    },
    /// A rule reads an `axioval:measured` name the registry
    /// (`axioval_ir::measured`) refuses: unknown, or with a wrong parameter.
    #[error("rule `{rule}` reads measured value `{property}`: {detail}")]
    InvalidMeasured {
        rule: String,
        property: String,
        detail: String,
    },
    /// A derived value of the ruleset is malformed, ill-typed, reads an
    /// undeclared value, or takes part in a cycle of values.
    #[error("value `{value}`: {detail}")]
    InvalidValue { value: String, detail: String },
    /// A rule's expression parameter is malformed or ill-typed.
    #[error("rule `{rule}` parameter `{parameter}`: {detail}")]
    InvalidExpression {
        rule: String,
        parameter: String,
        /// The path into the expression, such as `requirement.and[2]`.
        path: String,
        detail: String,
    },
    /// One rule reported two tables of one name.
    #[error("rule `{rule}` reported table `{table}` twice")]
    DuplicateReportTable { rule: String, table: String },
    /// A rule refines its outcomes in a way the package or the capability
    /// cannot support, such as severity bands on a capability that reports
    /// no deviation.
    #[error("rule `{rule}`: {detail}")]
    InvalidRefinement { rule: String, detail: String },
    /// The host asked the runtime to refine every outcome (locate it) and
    /// registered no outcome refiner to do so.
    #[error("{0} needs an outcome refiner, and the host registered none")]
    MissingRefiner(&'static str),
    /// A rule's gate or `ruleOutcome` selector names no rule of its
    /// ruleset, a cycle of rules depends on itself, or the host registered
    /// no outcome refiner to read another rule's selection.
    #[error("rule `{rule}`: {detail}")]
    InvalidDependency { rule: String, detail: String },
    /// A classification the ruleset derives is malformed, reads a rule's
    /// outcome, depends on itself through other classifications, is declared
    /// twice with different rows, or the host registered no outcome refiner
    /// to evaluate its rows.
    #[error("classification `{classification}`: {detail}")]
    InvalidClassification {
        classification: String,
        detail: String,
    },
    /// A grouping the ruleset derives is malformed, reads a rule's outcome
    /// or another grouping's groups, is declared twice with different
    /// definitions, or the host registered no outcome refiner to select its
    /// members.
    #[error("grouping `{grouping}`: {detail}")]
    InvalidGrouping { grouping: String, detail: String },
    /// A relation the ruleset declares is malformed, reads a rule's
    /// outcome or a declared relation, lists pairs that are not text,
    /// is declared twice with different definitions, is supplied pairs it
    /// does not take or rows that are not text pairs, or the host
    /// registered no outcome refiner to select its objects.
    #[error("relation `{relation}`: {detail}")]
    InvalidRelation { relation: String, detail: String },
    /// A catalogue the ruleset declares is malformed (its id, its key or
    /// its columns), is declared twice with different definitions, or the
    /// host supplied entries for one no ruleset declares, twice, or whose
    /// rows its declaration refuses.
    #[error("catalogue `{catalogue}`: {detail}")]
    InvalidCatalogue { catalogue: String, detail: String },
    /// The host supplied context zones whose id another supplied file
    /// already states.
    #[error("context zones `{origin}`: {detail}")]
    InvalidContextZones { origin: String, detail: String },
    /// The host stated the revision of a checked model twice (#323).
    #[error("model revision of {source_id}: {detail}")]
    InvalidModelRevision { source_id: String, detail: String },
}

pub use schema::ColumnKind;

/// One trusted column of a table parameter.
///
/// A definition's columns must match the descriptor's by ID, kind and
/// requirement, in any order; their names and descriptions are presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableColumn {
    pub id: &'static str,
    pub kind: ColumnKind,
    pub required: bool,
}
impl TableColumn {
    /// A column every row must fill.
    #[must_use]
    pub const fn required(id: &'static str, kind: ColumnKind) -> Self {
        Self {
            id,
            kind,
            required: true,
        }
    }
    /// A column a row may leave empty.
    #[must_use]
    pub const fn optional(id: &'static str, kind: ColumnKind) -> Self {
        Self {
            id,
            kind,
            required: false,
        }
    }
}

/// Supported declarative parameter types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterType {
    Boolean,
    Integer,
    Number,
    String,
    Quantity,
    Enum,
    /// An ISO 8601 calendar date, validated when the package is read.
    Date,
    /// An ISO 8601 date-time with a UTC offset, validated when the package
    /// is read.
    DateTime,
    Reference,
    ObjectTypeReference,
    PropertyReference,
    Selector,
    /// An expression whose value must be a truth, type checked when the
    /// ruleset is compiled.
    Expression,
    /// An expression whose value must be a plain number, which the
    /// capability evaluates per object itself (a finding's graded
    /// deviation); written with the package kind `expression`.
    NumberExpression,
    StringList,
    ReferenceList,
    /// Rows of typed cells in the given columns.
    Table(&'static [TableColumn]),
}
impl ParameterType {
    /// The kind's spelling in a definition package.
    #[must_use]
    pub fn package_kind(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::String => "string",
            Self::Quantity => "quantity",
            Self::Enum => "enum",
            Self::Date => "date",
            Self::DateTime => "dateTime",
            Self::Reference => "reference",
            Self::ObjectTypeReference => "objectTypeReference",
            Self::PropertyReference => "propertyReference",
            Self::Selector => "selector",
            Self::Expression | Self::NumberExpression => "expression",
            Self::StringList => "stringList",
            Self::ReferenceList => "referenceList",
            Self::Table(_) => "table",
        }
    }
    fn accepts(self, value: &schema::ParameterValue) -> bool {
        matches!(
            (self, value),
            (Self::Boolean, schema::ParameterValue::Boolean { .. })
                | (Self::Integer, schema::ParameterValue::Integer { .. })
                | (Self::Number, schema::ParameterValue::Number { .. })
                | (Self::String, schema::ParameterValue::String { .. })
                | (Self::Quantity, schema::ParameterValue::Quantity { .. })
                | (Self::Enum, schema::ParameterValue::Enum { .. })
                | (Self::Date, schema::ParameterValue::Date { .. })
                | (Self::DateTime, schema::ParameterValue::DateTime { .. })
                | (Self::Reference, schema::ParameterValue::Reference { .. })
                | (
                    Self::ObjectTypeReference,
                    schema::ParameterValue::ObjectTypeReference { .. }
                )
                | (
                    Self::PropertyReference,
                    schema::ParameterValue::PropertyReference { .. }
                )
                | (Self::Selector, schema::ParameterValue::Selector { .. })
                | (
                    Self::Expression | Self::NumberExpression,
                    schema::ParameterValue::Expression { .. }
                )
                | (Self::StringList, schema::ParameterValue::StringList { .. })
                | (
                    Self::ReferenceList,
                    schema::ParameterValue::ReferenceList { .. }
                )
                | (Self::Table(_), schema::ParameterValue::Table { .. })
        )
    }
}
/// Trusted capability parameter descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterDescriptor {
    pub name: String,
    pub parameter_type: ParameterType,
    pub required: bool,
    /// Whether a rule may give the parameter as an expression evaluated per
    /// checked object, or, for a table, any scalar cell. Every other
    /// parameter is constant for the whole rule and refuses one.
    pub per_object: bool,
    /// Whether a `string` parameter holds an expression's text form, which
    /// a rule may also give as the expression itself; the capability parses
    /// and checks it.
    pub expression_text: bool,
}
impl ParameterDescriptor {
    /// Required parameter descriptor.
    pub fn required(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: true,
            per_object: false,
            expression_text: false,
        }
    }
    /// Optional parameter descriptor.
    pub fn optional(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: false,
            per_object: false,
            expression_text: false,
        }
    }
    /// The `string` parameter holds an expression's text form, and a rule
    /// may give the expression itself instead.
    #[must_use]
    pub fn expression_text(mut self) -> Self {
        self.expression_text = true;
        self
    }
    /// The parameter may be computed per checked object: given as an
    /// expression of its kind or, for a table, with expression cells.
    #[must_use]
    pub fn per_object(mut self) -> Self {
        self.per_object = true;
        self
    }
}

/// One validated portable rule bound to trusted executable capability code.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledRule {
    /// Package-local stable rule ID.
    pub id: RuleId,
    /// Registered capability ID.
    pub capability: String,
    /// Rule severity.
    pub severity: schema::Severity,
    /// Source-neutral applicability selector.
    pub selector: schema::Selector,
    /// Strictly validated parameter bindings.
    pub parameters: BTreeMap<String, schema::ParameterValue>,
}

/// Source-neutral data and typed host services visible during one rule evaluation.
pub struct RuleContext<'a> {
    /// Immutable composed project view.
    pub project: &'a Project,
    /// Adapter-provided semantic and computational capabilities.
    pub services: &'a ServiceRegistry,
}

/// Fail-closed output from one trusted capability evaluation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapabilityEvaluation {
    findings: Vec<Finding>,
    not_evaluated: Vec<CapabilityNotEvaluated>,
    tables: Vec<ReportTable>,
    /// The deviation of each graded finding, by its index in `findings`.
    graded: Vec<(usize, Deviation)>,
}
/// A not-evaluated outcome before the runtime binds its compiled rule ID.
#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityNotEvaluated {
    scope: Scope,
    reason: NotEvaluatedReason,
    message: String,
    location: Option<Location>,
    explanation: Option<axioval_ir::Explanation>,
}
impl CapabilityNotEvaluated {
    /// The object that could not be evaluated, if the outcome is about one.
    #[must_use]
    pub fn object_id(&self) -> Option<&ObjectId> {
        self.scope.object()
    }
    /// What could not be evaluated: an object, a source, or the whole rule.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    #[must_use]
    pub fn reason(&self) -> &NotEvaluatedReason {
        &self.reason
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Where the outcome's object lies, once an [`OutcomeRefiner`] located
    /// it.
    #[must_use]
    pub fn location(&self) -> Option<&Location> {
        self.location.as_ref()
    }
    /// Locates the outcome; for an [`OutcomeRefiner`].
    pub fn set_location(&mut self, location: Location) {
        self.location = Some(location);
    }
}

impl CapabilityEvaluation {
    /// Adds every outcome of `other`: its findings with their deviations,
    /// its not-evaluated outcomes and its tables.
    pub fn absorb(&mut self, other: Self) {
        let offset = self.findings.len();
        self.findings.extend(other.findings);
        self.graded.extend(
            other
                .graded
                .into_iter()
                .map(|(index, deviation)| (index + offset, deviation)),
        );
        self.not_evaluated.extend(other.not_evaluated);
        self.tables.extend(other.tables);
    }
    /// Conclusive findings emitted by this capability.
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Explicit fail-closed outcomes emitted by this capability.
    #[must_use]
    pub fn not_evaluated_outcomes(&self) -> &[CapabilityNotEvaluated] {
        &self.not_evaluated
    }
    /// Tables of measured values emitted by this capability.
    #[must_use]
    pub fn tables(&self) -> &[ReportTable] {
        &self.tables
    }
    /// Creates a conclusive evaluation from zero or more findings.
    #[must_use]
    pub fn evaluated(findings: Vec<Finding>) -> Self {
        Self {
            findings,
            ..Self::default()
        }
    }
    /// Adds a table of measured values, reported beside the findings.
    ///
    /// A table without rows is dropped: it measured nothing, and the
    /// not-evaluated outcomes already say why. Tables are informative only;
    /// they never stand in for a finding or a not-evaluated outcome.
    pub fn push_table(&mut self, table: ReportTable) {
        if !table.is_empty() {
            self.tables.push(table);
        }
    }
    /// Creates a rule-level not-evaluated outcome.
    #[must_use]
    pub fn not_evaluated(reason: NotEvaluatedReason, message: impl Into<String>) -> Self {
        let mut outcome = Self::default();
        outcome.push_not_evaluated(reason, message);
        outcome
    }
    /// Adds a conclusive finding, about an object, a source or the project
    /// as its [`Scope`] says.
    pub fn push_finding(&mut self, finding: Finding) {
        self.findings.push(finding);
    }
    /// Adds a finding of a value missing its bound by `deviation`, so a rule
    /// declaring severity bands grades it. The finding keeps its own
    /// severity otherwise; the deviation is never reported.
    pub fn push_graded_finding(&mut self, finding: Finding, deviation: Deviation) {
        self.graded.push((self.findings.len(), deviation));
        self.findings.push(finding);
    }
    /// Adds a finding, graded when `deviation` is known.
    pub fn push_finding_deviating(&mut self, finding: Finding, deviation: Option<Deviation>) {
        match deviation {
            Some(deviation) => self.push_graded_finding(finding, deviation),
            None => self.push_finding(finding),
        }
    }
    /// The deviation a graded finding was pushed with, by its index in
    /// [`Self::findings`]; `None` for an ungraded one.
    #[must_use]
    pub fn deviation(&self, finding: usize) -> Option<Deviation> {
        self.graded
            .iter()
            .find(|(index, _)| *index == finding)
            .map(|(_, deviation)| *deviation)
    }
    /// Grades every graded finding's severity by `bands`, the finding's own
    /// severity standing beyond the last band.
    fn grade(&mut self, bands: &[schema::SeverityBand]) {
        for (index, deviation) in std::mem::take(&mut self.graded) {
            let finding = &mut self.findings[index];
            let (severity, mixed) = refinement::grade(bands, &finding.severity, deviation);
            if mixed {
                use std::fmt::Write as _;
                let _ = write!(
                    finding.message,
                    "; deviation between {} and {}, graded {} by its most severe band",
                    percent(deviation.lower()),
                    percent(deviation.upper()),
                    refinement::label(&severity)
                );
            }
            finding.severity = severity;
        }
    }
    /// Findings, for an [`OutcomeRefiner`] to refine in place.
    pub fn findings_mut(&mut self) -> &mut [Finding] {
        &mut self.findings
    }
    /// Not-evaluated outcomes, for an [`OutcomeRefiner`] to locate.
    pub fn not_evaluated_outcomes_mut(&mut self) -> &mut [CapabilityNotEvaluated] {
        &mut self.not_evaluated
    }
    /// Removes and returns every finding, for an [`OutcomeRefiner`] to put
    /// back refined; deviations are dropped with them.
    pub fn take_findings(&mut self) -> Vec<Finding> {
        self.graded.clear();
        std::mem::take(&mut self.findings)
    }
    /// Adds a not-evaluated outcome about `scope`: an object, a source, or
    /// the rule as a whole.
    pub fn push_not_evaluated_about(
        &mut self,
        scope: Scope,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.push_unavailable(scope, reason, message);
    }
    /// Adds a rule-level not-evaluated outcome.
    pub fn push_not_evaluated(&mut self, reason: NotEvaluatedReason, message: impl Into<String>) {
        self.push_unavailable(Scope::Project, reason, message);
    }
    /// Adds a not-evaluated outcome about one source as a whole, such as a
    /// count over that source that undecided objects could still change.
    pub fn push_source_not_evaluated(
        &mut self,
        source: SourceId,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.push_unavailable(Scope::Source(source), reason, message);
    }
    /// Adds an object-specific not-evaluated outcome.
    pub fn push_object_not_evaluated(
        &mut self,
        object_id: ObjectId,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.push_unavailable(Scope::Object(object_id), reason, message);
    }
    /// An object left not evaluated, with how an expression left it open.
    pub fn push_object_not_evaluated_explained(
        &mut self,
        object_id: ObjectId,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
        explanation: axioval_ir::Explanation,
    ) {
        self.push_unavailable(Scope::Object(object_id), reason, message);
        if let Some(outcome) = self.not_evaluated.last_mut() {
            outcome.explanation = Some(explanation);
        }
    }
    fn push_unavailable(
        &mut self,
        scope: Scope,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.not_evaluated.push(CapabilityNotEvaluated {
            scope,
            reason,
            message: message.into(),
            location: None,
            explanation: None,
        });
    }
}

/// A relative deviation as a reviewer reads it, `12.5 %`.
fn percent(value: f64) -> String {
    if value.is_finite() {
        format!("{} %", (value * 1e4).round() / 1e2)
    } else {
        "unbounded".to_owned()
    }
}

/// Trusted code selected by a package capability ID; packages never supply executable code.
pub trait RuleCapability: Send + Sync {
    /// Stable trusted capability ID.
    fn id(&self) -> &'static str;
    /// Strict accepted parameters.
    fn parameters(&self) -> Vec<ParameterDescriptor>;
    /// Whether findings report how far a value misses its bound
    /// ([`CapabilityEvaluation::push_graded_finding`]), so a rule may grade
    /// them with severity bands. A rule declaring bands on a capability that
    /// answers `false` fails compilation.
    fn grades_deviation(&self) -> bool {
        false
    }
    /// Whether a rule definition may declare parameters beyond
    /// [`RuleCapability::parameters`]: scalar values, string lists and
    /// tables with the columns the definition declares, which the rule's
    /// expressions read. The compiler checks each against its declared kind
    /// and columns; the capability's own parameters stay strict.
    fn takes_authored_parameters(&self) -> bool {
        false
    }
    /// What `rule` may ask of a host's geometry ([`GeometryReach`]), so a
    /// host meshes only what the plan's rules can measure (#410).
    /// [`GeometryReach::Unbounded`] unless the capability states less: a
    /// capability stating less than it asks would be measured on missing
    /// bodies, so state [`GeometryReach::None`] only for code that asks no
    /// geometry service, and [`GeometryReach::Selected`] only for code that
    /// measures the bodies of those selections alone.
    fn geometry_reach(&self, rule: &CompiledRule) -> GeometryReach {
        let _ = rule;
        GeometryReach::Unbounded
    }
    /// Evaluates an already-validated rule request.
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation;
    /// The composition the capability is built as, when it is a
    /// [`template::Template`]: what the catalogue shows and a rule bound to
    /// it may be forked from. `None` for a capability implemented in code.
    fn template(&self) -> Option<&template::Template> {
        None
    }
}

/// Host-controlled registry of trusted capabilities.
#[derive(Clone, Default)]
pub struct CapabilityRegistry {
    capabilities: BTreeMap<String, Arc<dyn RuleCapability>>,
    refiner: Option<Arc<dyn OutcomeRefiner>>,
    measured: Vec<Arc<dyn MeasuredProvider>>,
    /// Each measured name's provider, by its position in `measured`: a run
    /// finds a value's provider at every read.
    measured_index: measured::provider::ProviderIndex,
}
impl CapabilityRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers a capability; duplicate IDs are rejected.
    pub fn register<C: RuleCapability + 'static>(
        mut self,
        capability: C,
    ) -> Result<Self, EngineError> {
        let id = capability.id().to_owned();
        if self
            .capabilities
            .insert(id.clone(), Arc::new(capability))
            .is_some()
        {
            return Err(EngineError::DuplicateCapability(id));
        }
        Ok(self)
    }
    /// Replaces the capability registered under `capability`'s ID with
    /// `capability`: a host running a rebuilt capability's replaced
    /// implementation in its place, as the template benchmark does, with
    /// everything else registered unchanged.
    ///
    /// # Errors
    ///
    /// [`EngineError::UnknownCapability`] when nothing is registered under
    /// the ID: a replacement never registers a capability of its own.
    pub fn replace<C: RuleCapability + 'static>(
        mut self,
        capability: C,
    ) -> Result<Self, EngineError> {
        let id = capability.id().to_owned();
        match self.capabilities.get_mut(&id) {
            Some(registered) => *registered = Arc::new(capability),
            None => return Err(EngineError::UnknownCapability(id)),
        }
        Ok(self)
    }
    /// Registers trusted code measuring registered values
    /// ([`MeasuredProvider`]). Each name must be in the registry of measured
    /// values and measured by neither the engine nor another provider.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidProvider`] naming the first name refused.
    pub fn register_measured<P: MeasuredProvider>(
        mut self,
        provider: P,
    ) -> Result<Self, EngineError> {
        for name in provider.names() {
            let refused = |why: &str| Err(EngineError::InvalidProvider((*name).into(), why.into()));
            if axioval_ir::measured::descriptor(name).is_none_or(|known| known.name != *name) {
                return refused("it is not in the registry of measured values");
            }
            if measured::measured_by_core(name) {
                return refused("the engine measures it");
            }
            if self
                .measured
                .iter()
                .any(|other| other.names().contains(name))
            {
                return refused("another provider measures it");
            }
        }
        for name in provider.member_lists() {
            let refused = |why: &str| Err(EngineError::InvalidProvider((*name).into(), why.into()));
            if axioval_ir::measured::member_descriptor(name)
                .is_none_or(|known| known.list.name != *name)
            {
                return refused("it is not in the registry of measured member lists");
            }
            if self
                .measured
                .iter()
                .any(|other| other.member_lists().contains(name))
            {
                return refused("another provider measures it");
            }
        }
        let position = self.measured.len();
        let index = Arc::make_mut(&mut self.measured_index);
        for name in provider.names() {
            index.insert(name, position);
        }
        self.measured.push(Arc::new(provider));
        Ok(self)
    }
    /// What each registered provider measures, in registration order: its
    /// names and its member lists. Tests hold every provider to a contract
    /// through it, so a new one is never left out.
    pub fn measured_providers(
        &self,
    ) -> impl Iterator<Item = (&'static [&'static str], &'static [&'static str])> + '_ {
        self.measured
            .iter()
            .map(|provider| (provider.names(), provider.member_lists()))
    }
    /// Whether a registered provider measures the member list `name`.
    #[must_use]
    pub fn measures_members(&self, name: &str) -> bool {
        self.measured
            .iter()
            .any(|provider| provider.member_lists().contains(&name))
    }
    /// Whether a registered provider or the engine measures `name`.
    #[must_use]
    pub fn measures(&self, name: &str) -> bool {
        measured::measured_by_core(name)
            || self
                .measured
                .iter()
                .any(|provider| provider.names().contains(&name))
    }
    /// Installs the registered measured-value providers into `services`
    /// for `project`, as a run does, so [`measured_value`] reads what a run
    /// would.
    pub fn install_measured(&self, services: &mut ServiceRegistry, project: &Project) {
        measured::provider::install(services, &self.measured, &self.measured_index, project);
    }
    /// Every registered capability ID, sorted.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.capabilities.keys().map(String::as_str)
    }
    /// Gets trusted code by exact ID.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn RuleCapability>> {
        self.capabilities.get(id)
    }
    /// Installs the trusted code that applies rule refinements reading the
    /// model, replacing any installed before.
    #[must_use]
    pub fn with_refiner<R: OutcomeRefiner + 'static>(mut self, refiner: R) -> Self {
        self.refiner = Some(Arc::new(refiner));
        self
    }
    /// The installed outcome refiner, if any.
    pub fn refiner(&self) -> Option<&Arc<dyn OutcomeRefiner>> {
        self.refiner.as_ref()
    }
}

/// A compiled rule the engine cannot execute as authored.
///
/// Carried in the plan so every run reports it as not evaluated; dropping it
/// would make an unexecuted requirement indistinguishable from a satisfied one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeferredRule {
    /// Package-local stable rule ID.
    pub id: RuleId,
    /// Registered capability ID.
    pub capability: String,
    /// Why the rule cannot run as authored.
    pub reason: String,
}

/// Validated, deterministic request plan.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    rules: Vec<CompiledRule>,
    /// The rules in the plan's own order where [`Self::evaluate_in`] evaluates
    /// them in another: the order the evaluation budget's backstop refuses
    /// them in (#447).
    planned: Option<Arc<[RuleId]>>,
    deferred: Vec<DeferredRule>,
    concepts: Arc<ConceptCatalog>,
    refinements: BTreeMap<RuleId, RuleRefinement>,
    /// Whole-rule gates: each gated rule's parents and conditions.
    gates: BTreeMap<RuleId, Vec<(RuleId, schema::GateCondition)>>,
    /// Rules whose selection a dependent reads per object.
    recorded: BTreeSet<RuleId>,
    /// Rules reported only through the rules that read them.
    auxiliary: BTreeSet<RuleId>,
    /// Classifications to derive before any rule runs, each after those
    /// its rows read.
    classifications: Vec<schema::ClassificationDefinition>,
    /// Groupings to derive after the classifications, before any rule runs.
    groupings: Vec<schema::GroupingDefinition>,
    /// Relations to derive after the groupings, before any rule runs, their
    /// listed pairs inline.
    relations: Vec<schema::RelationDefinition>,
    /// The pairs the host supplied for its supplied relations, by id.
    supplied: BTreeMap<String, SuppliedPairs>,
    /// The catalogues the rulesets declare, with the entries the host
    /// supplied for them.
    catalogues: Catalogues,
    /// The context zones the host supplied beside the model.
    zones: SuppliedZones,
    /// The values the ruleset derives, by name.
    values: Arc<BTreeMap<String, schema::Expression>>,
    /// The types the ruleset declares for its expressions' reads.
    types: Arc<expression::DeclaredTypes>,
    /// The rules each rule reads the outcomes of, through gates,
    /// `ruleOutcome` selectors and severity overrides: what any order of
    /// evaluation must keep ([`Self::evaluate_in`]).
    dependencies: BTreeMap<RuleId, BTreeSet<RuleId>>,
}

/// The order a plan's rules are evaluated in, as far as their
/// dependencies leave it free ([`ExecutionPlan::evaluate_in`]).
///
/// A report never depends on it: every rule is a function of the model,
/// its parameters and the outcomes it reads, and whatever one rule builds
/// for another is a pure function of its key, charged as if built
/// (ADR 0006). The determinism gate (#441) evaluates a check in several
/// orders and requires the same bytes, under an expression evaluation
/// budget that runs out too (#447): each rule spends a share of its own,
/// and the run-wide backstop refuses rules in the plan's order whatever
/// the order they ran in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RuleOrder {
    /// The plan's own: by rule id, after the rules each reads.
    #[default]
    Plan,
    /// The plan's reversed: the last rule first, unless it reads another.
    Reversed,
    /// A shuffle of the plan's, the same for the same seed.
    Shuffled(u64),
}

impl ExecutionPlan {
    /// Rules in execution order: every rule after the rules its gates and
    /// `ruleOutcome` selectors read, and otherwise by stable rule ID.
    pub fn rules(&self) -> &[CompiledRule] {
        &self.rules
    }
    /// The same plan evaluating its rules in `order` wherever their
    /// dependencies leave it free: each rule still after every rule whose
    /// outcomes it reads. Only to show that the order changes nothing
    /// (#441); the plan's own order is the one a check uses.
    #[must_use]
    pub fn evaluate_in(mut self, order: RuleOrder) -> Self {
        if self.planned.is_none() {
            self.planned = Some(self.rules.iter().map(|rule| rule.id.clone()).collect());
        }
        let mut preferred = std::mem::take(&mut self.rules);
        match order {
            RuleOrder::Plan => {}
            RuleOrder::Reversed => preferred.reverse(),
            RuleOrder::Shuffled(seed) => rule_outcomes::shuffle(&mut preferred, seed),
        }
        self.rules = rule_outcomes::preferred_order(preferred, &self.dependencies);
        self
    }
    /// Rules compiled but not executable as authored, ordered by rule ID.
    pub fn deferred(&self) -> &[DeferredRule] {
        &self.deferred
    }
    /// Canonical concepts declared by the ruleset's definition packages.
    pub fn concepts(&self) -> &ConceptCatalog {
        &self.concepts
    }
    /// How `rule` refines its outcomes; `None` when it declares nothing.
    pub fn refinement(&self, rule: &RuleId) -> Option<&RuleRefinement> {
        self.refinements.get(rule)
    }
    /// The classifications the plan derives, in the order they are derived.
    pub fn classifications(&self) -> &[schema::ClassificationDefinition] {
        &self.classifications
    }
    /// The groupings the plan derives, by id.
    pub fn groupings(&self) -> &[schema::GroupingDefinition] {
        &self.groupings
    }
    /// The relations the plan derives, by id, their listed pairs inline.
    pub fn relations(&self) -> &[schema::RelationDefinition] {
        &self.relations
    }
    /// The pairs the host supplied for the supplied relation `id`, if any.
    pub fn supplied_pairs(&self, id: &str) -> Option<&SuppliedPairs> {
        self.supplied.get(id)
    }
    /// The catalogues the plan's expressions may read, by id.
    pub fn catalogues(&self) -> Vec<&schema::CatalogueDefinition> {
        self.catalogues.definitions().collect()
    }
    /// The entries the host supplied for the catalogue `id`, if any.
    pub fn supplied_catalogue(&self, id: &str) -> Option<&SuppliedCatalogue> {
        self.catalogues.supplied(id)
    }
    /// Supplies the entries of the catalogue `id` for this run. A catalogue
    /// the host supplies no entries for answers no key: every object whose
    /// expression reads it is not evaluated.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidCatalogue`] when no ruleset declares `id`, its
    /// entries were already supplied, its version is blank, or an entry
    /// fills an undeclared column, holds a cell not of its column's kind,
    /// names no key or a key another entry names.
    pub fn supply_catalogue(
        &mut self,
        id: &str,
        catalogue: SuppliedCatalogue,
    ) -> Result<(), EngineError> {
        self.catalogues
            .supply(id, catalogue)
            .map_err(|detail| EngineError::InvalidCatalogue {
                catalogue: id.to_owned(),
                detail,
            })
    }
    /// Whether a catalogue the rulesets declare holds the results of an
    /// external calculation, whose reads need the checked models'
    /// revisions ([`Self::supply_model_revision`]).
    pub fn reads_calculation_results(&self) -> bool {
        self.catalogues.has_results()
    }
    /// States the revision of the checked model `source` (#323): a result
    /// of an external calculation is read for its objects only where it
    /// was computed on this revision. A model whose revision is not stated
    /// matches no result.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidModelRevision`] when the revision of `source`
    /// was already stated.
    pub fn supply_model_revision(
        &mut self,
        source: SourceId,
        revision: ModelRevision,
    ) -> Result<(), EngineError> {
        let source_id = source.to_string();
        self.catalogues
            .supply_revision(source, revision)
            .map_err(|detail| EngineError::InvalidModelRevision { source_id, detail })
    }
    /// The revision stated for the checked model `source`, if any.
    pub fn model_revision(&self, source: &SourceId) -> Option<&ModelRevision> {
        self.catalogues.revision(source)
    }
    /// Every supplied result of an external calculation the run will not
    /// read: computed on a revision no checked model has (stale), or
    /// stating no method, report, date or revision (untraceable), by
    /// catalogue and then the file's order. Reading any of them is not
    /// evaluated; a host reports them as integrity warnings.
    pub fn stale_results(&self) -> Vec<StaleResult> {
        self.catalogues.stale_results()
    }
    /// Supplies the pairs of the relation `id`, declared `by: supplied`,
    /// for this run. A supplied relation the host supplies no pairs for
    /// relates nothing surely: every rule walking it is not evaluated.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidRelation`] when no ruleset declares `id`, its
    /// ruleset states its pairs itself, its pairs were already supplied, or
    /// a row is not exactly a non-blank text `from` and `to`.
    pub fn supply_relation(&mut self, id: &str, pairs: SuppliedPairs) -> Result<(), EngineError> {
        let invalid = |detail: String| EngineError::InvalidRelation {
            relation: id.to_owned(),
            detail,
        };
        let Some(definition) = self.relations.iter().find(|known| known.id == id) else {
            return Err(invalid(
                "no ruleset declares it, so its pairs cannot be supplied".into(),
            ));
        };
        if !matches!(definition.by, schema::RelationKey::Supplied { .. }) {
            return Err(invalid(
                "its ruleset states its pairs itself, so they cannot be supplied".into(),
            ));
        }
        if self.supplied.contains_key(id) {
            return Err(invalid("its pairs are supplied twice".into()));
        }
        compiler::check_pair_rows(pairs.rows())
            .map_err(|detail| invalid(format!("the pairs of `{}`: {detail}", pairs.origin())))?;
        self.supplied.insert(id.to_owned(), pairs);
        Ok(())
    }
    /// The whole-rule gates of `rule`: each parent rule and the condition
    /// on its outcome. Empty for an ungated rule.
    pub fn gates(&self, rule: &RuleId) -> &[(RuleId, schema::GateCondition)] {
        self.gates.get(rule).map_or(&[], Vec::as_slice)
    }
    /// Whether `rule` is auxiliary: run for the rules that read its
    /// outcome, and never reported itself.
    pub fn is_auxiliary(&self, rule: &RuleId) -> bool {
        self.auxiliary.contains(rule)
    }
    /// Supplies the zones of one file for this run (#327). Zones are
    /// context beside the model, never part of a rule package: every
    /// measured value naming a zone no file supplies is not evaluated.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidContextZones`] when a zone's id is already
    /// supplied by an earlier file.
    pub fn supply_context_zones(&mut self, zones: ContextZones) -> Result<(), EngineError> {
        let origin = zones.origin().to_owned();
        self.zones
            .supply(zones)
            .map_err(|detail| EngineError::InvalidContextZones { origin, detail })
    }
    /// The context zones supplied for this run.
    pub fn context_zones(&self) -> &SuppliedZones {
        &self.zones
    }
    /// The context zones the plan's rules and values read through
    /// measured names: each `zone` stated or bound from a rule's text
    /// parameter, and whether any measured value reads a zone at all.
    pub fn context_zone_reads(&self) -> ZoneReads {
        let mut reads = ZoneReads::default();
        for rule in &self.rules {
            let parameters = |name: &str| match rule.parameters.get(name) {
                Some(schema::ParameterValue::String { value }) => Some(value.clone()),
                _ => None,
            };
            if let Ok(value) = serde_json::to_value((&rule.parameters, &rule.selector)) {
                context_zones::zone_reads(&value, &parameters, &mut reads);
            }
        }
        if let Ok(value) = serde_json::to_value(self.values.as_ref()) {
            context_zones::zone_reads(&value, &|_| None, &mut reads);
        }
        reads
    }
}

mod alignment;
mod alignment_element;
mod boundary_coverage;
pub mod catalogue;
mod catalogues;
mod circulation;
mod classifications;
mod compartments;
mod compiler;
mod concepts;
mod contact;
mod context_zones;
mod coordinate_system;
mod corridor_end;
mod coverage;
pub mod derivation;
mod derived;
mod derived_relationships;
mod discipline_map;
mod door_leaves;
pub mod draft;
mod envelope_membership;
mod expression_binding;
mod facade_area;
mod federation;
mod free_space;
mod geometry_demand;
mod groupings;
mod guard;
mod integrity;
mod leaf_glazing;
mod linear_quantity;
mod local_origin;
mod material_layers;
mod measured;
pub mod memory;
mod metric_routing;
mod network;
mod object_frame;
mod pairwise;
pub mod parallel;
mod path;
mod plan_area;
mod plan_region;
mod plan_span;
mod properties;
mod proximity;
mod refinement;
mod relations;
mod relationships;
mod resources;
mod rule_outcomes;
mod section;
mod services;
mod setback;
mod side_distance;
mod sight;
mod sight_distance;
mod source_metadata;
mod space;
mod station_profile;
mod table_files;
pub mod template;
mod terrain;
mod timing;
mod topology;
mod triangle_count;
mod values;
mod vertical_extent;
mod walkability;
mod walking_surface;
mod zone_area;
pub use alignment::{
    AlignmentError, AlignmentInterval, AlignmentParameter, AlignmentParameterRequest,
    AlignmentParameterValue, AlignmentPosition, AlignmentRequest, AlignmentService,
    AlignmentServiceHandle,
};
pub use alignment_element::{
    AlignmentElement, AlignmentElementKind, AlignmentElements, AlignmentElementsRequest,
    AlignmentLayout, ElementEnd,
};
pub use boundary_coverage::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryCoverageServiceHandle, BoundaryOverlap, BoundaryPlacement, CoverageAreas,
    MeasuredBoundary, ShareInterval, SurfaceAreaInterval,
};
pub use catalogues::{
    CATALOGUE_LOCATOR_PREFIX, Catalogues, ModelRevision, StaleResult, SuppliedCatalogue,
};
pub use circulation::{
    CirculationContact, CirculationMap, CirculationNode, CirculationNodeKind, CirculationRequest,
};
pub use classifications::{
    ClassificationAssignment, ClassificationError, ClassificationService,
    ClassificationServiceHandle,
};
pub use compiler::{QUALIFIED_RULE_SEPARATOR, SUPPORTED_SCHEMA_VERSION, compile, compile_rulesets};
pub use concepts::{
    BindingError, ConceptBindings, ConceptCatalog, ConceptKind, ObjectNature, TypeHierarchyError,
    TypeHierarchyService, TypeHierarchyServiceHandle, binding_reason, bound_property_request,
};
pub use contact::{
    ContactError, ContactEvidence, ContactRequest, ContactService, ContactServiceHandle,
    ContactSide, ContactTolerance,
};
pub use context_zones::{
    ContextZone, ContextZones, MODEL_REFERENCE, PlacedZone, SuppliedZones, ZONE_LOCATOR_PREFIX,
    ZoneReads, ZoneReference, ZoneShape, ZoneValue, reads_zone,
};
pub use coordinate_system::{
    CoordinateFrame, CoordinateSystemError, CoordinateSystemService, CoordinateSystemServiceHandle,
    MAP_SCALE_TOLERANCE, MapConversion, MapPlacement, MapPlacementRefusal, SitePlacement,
    SourceCoordinateSystem, same_reference_system,
};
pub use corridor_end::{CorridorEnd, CorridorEndRequest, CorridorEnds, EndWall, WallContact};
pub use coverage::{CoverageEvidence, CoverageRequest, EffectMeets, EffectReach, Participant};
pub use derivation::{
    DerivationError, DerivationKind, DerivationStatistics, DerivationStore, DerivationVersion,
    FastBudget, Inputs, Kernel, KeyCompleteness, Precision,
};
pub use derived::{ClassOutcome, Classifications};
pub use derived_relationships::{
    AdjacentSide, DERIVATIONS, DERIVED_RELATIONSHIP_PREFIX, Derivation, DerivationEntry,
    DerivationParameter, DerivedRelationshipService, DerivedRelationshipServiceHandle, LevelFacts,
    LevelMatch, across_face, across_side, across_stated, adjacent_side,
};
pub use discipline_map::{
    DisciplineMap, DisciplineMapError, DisciplineOrigin, DisciplineRule, UnmappedReason,
    wildcard_regex,
};
pub use door_leaves::{
    DoorLeaf, DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, PlanRing,
    SWEPT_FLOOR_REACH_METRES, SweptDoor, SwingSector, UnplacedLeaf,
};
pub use envelope_membership::{
    EnvelopeDerivation, EnvelopeMembershipError, EnvelopeMembershipEvidence,
    EnvelopeMembershipRequest, EnvelopeMembershipService, EnvelopeMembershipServiceHandle,
};
pub use facade_area::{
    EnclosedVolume, FacadeArea, FacadeAreaError, FacadeAreaService, FacadeAreaServiceHandle,
    FacadeFace, FacadeFaceSide, FacadeFaces,
};
pub use free_space::{
    AreaInterval, BoxClearance, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest,
    ClearanceShape, CompleteClearanceEvidence, CompletePlacementEvidence, CompleteSupportEvidence,
    ContainmentEvidence, ContainmentOutcome, ContainmentRequest, CylinderClearance, ElevationBand,
    EntranceReach, FrameOffsetPlacement, FreeAreaEvidence, FreeAreaRequest, FreeFloorEvidence,
    FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle, LargestDiscEstimate, MetricDirection,
    MetricFrame, ObstructionEvidence, PlacementDomain, PlacementOrientation, PlacementOutcome,
    PlacementRequest, PlacementShape, SignedDistanceInterval, SupportCoverageEvidence,
    SupportCoverageOutcome, SupportCoverageRequest, SupportedPlacement,
};
pub use geometry_demand::{
    GeometryDemand, GeometryReach, geometry_demand, rule_selectors, services_asked,
};
pub use groupings::{
    DerivedGroups, GROUP_RELATIONSHIP_PREFIX, Group, Grouping, Membership, PropertyRead, group_id,
};
pub use guard::{
    ClimbableCandidate, GuardCandidate, GuardEdge, GuardError, GuardEvidence, GuardSearch,
    GuardService, GuardServiceHandle,
};
pub use integrity::{
    IntegrityError, IntegrityIssue, IntegritySeverity, SourceIntegrityService,
    SourceIntegrityServiceHandle,
};
pub use leaf_glazing::{
    DoorGlazing, LeafGlazing, LeafGlazingError, LeafGlazingService, LeafGlazingServiceHandle,
};
pub use linear_quantity::{
    LinearInterval, LinearQuantityError, LinearQuantityEvidence, LinearQuantityKind,
    LinearQuantityRequest, LinearQuantityService, LinearQuantityServiceHandle, ShelfGeometry,
};
pub use local_origin::{LocalOrigin, LocalOrigins};
pub use material_layers::{
    LayerAxis, LayerDirection, LayerSense, MaterialLayer, MaterialLayerError, MaterialLayerService,
    MaterialLayerServiceHandle, MaterialLayers,
};
pub use measured::elements::AlignmentElementMeasures;
pub use measured::faces::FacePieceMeasures;
pub use measured::layers::MaterialLayerMeasures;
pub use measured::provider::{
    ArgumentsKey, Citation, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement,
    MemberValue, measured_members, measured_members_bound, measured_members_cited,
};
pub use measured::{BoundRead, MeasuredRead, MeasuredValues, PreparedRead, measured_value};
pub use memory::MemoryBudget;
pub use metric_routing::{
    BlockedMetricRouteEvidence, ClimbLength, CompleteMetricEvidence, ConnectorRouting,
    FarthestPointEvidence, FarthestPointOutcome, FarthestPointRequest, ForcedWalkEvidence,
    ForcedWalkOutcome, ForcedWalkRequest, LengthInterval, MetricPoint, MetricRouteEvidence,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingError, MetricRoutingService,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetEvidence, NearestTargetOutcome,
    NearestTargetRequest, NeverEnteredEvidence, PathTrace, PathTraceRequest, StairLength,
    ThresholdVerdict, TravelCost, UnreachableRegionEvidence, UnreachableTargetsEvidence,
};
pub use network::{
    DistributionNetworkService, DistributionNetworkServiceHandle, FLOW_RELATIONSHIP, FlowGraph,
    FlowGraphs, NetworkServiceError,
};
pub use object_frame::{
    ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront,
};
pub use pairwise::{
    CandidatePair, CandidateSearchError, candidate_pairs, projected_candidate_pairs,
};
pub use parallel::Parallelism;
pub use path::{PathSegment, WALK_BUDGET};
pub use plan_area::{
    ElevationCover, ElevationRequest, FootprintOutline, FootprintPerimeter, PlanArea,
    PlanAreaError, PlanAreaService, PlanAreaServiceHandle, PlanBand,
};
pub use plan_region::ConvexPlanRegion;
pub use plan_span::{
    CentrePlacement, PlanCentre, PlanLength, PlanRecess, PlanRecesses, PlanRectangle, PlanSection,
    PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle, RectangleOrientation,
};
pub use properties::{
    CompletePropertyAbsenceEvidence, NameMatch, NamePattern, PropertyEnumeration,
    PropertyEnumerationRequest, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ResolvedProperty, UnreadableValue,
};
pub use proximity::{
    BodyContainment, BodySurface, BodyVolume, Bounds3, CounterpartSurface, DirectedDistance,
    DrainageEvidence, DrainagePath, DrainageRequest, ExactBoundaryHandle, FaceClass,
    FaceDistanceError, FaceDistanceEvidence, FaceDistanceRequest, GeometryFidelity, HeightAbove,
    HeightAboveEvidence, HeightAboveRequest, IntersectionVolume, LineDistanceEvidence,
    LineDistanceRequest, LineProjection, ObjectBounds, OverlapAlongEvidence, OverlapAlongRequest,
    OverlapExtents, ProjectedDistanceEvidence, ProximityError, ProximityEvidence,
    ProximityProjection, ProximityRequest, ProximityService, ProximityServiceHandle,
    RegionDistanceEvidence, RegionDistanceRequest, SubjectSurface, SurfaceBasis, SurfaceDirection,
    SurfaceDistanceEvidence, SurfaceDistanceRequest, VerticalDirection, VerticalSurfaces,
    VolumeInterval, WITNESS_ROUNDING_METRES,
};
pub use refinement::{
    Deviation, LocationMethod, LocationPolicy, OutcomeRefiner, Refining, RuleRefinement,
    report_severity,
};
pub use relations::{
    DeclaredRelations, RELATION_RELATIONSHIP_PREFIX, Relation, SuppliedPairs,
    UnknownRelationObject, unknown_relation_objects,
};
pub use relationships::{
    AbsentEndPolicy, CandidateUniverse, CompleteRelationshipEdges, CompleteRelationshipSelection,
    RELATIONSHIP_KIND_PREFIX, RelationshipEdge, RelationshipEdgesRequest, RelationshipKind,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, SemanticRelationship,
    TraversalDirection,
};
pub use resources::{
    Reached, ResourceError, ResourceObjects, ResourceRequest, ResourceService,
    ResourceServiceHandle,
};
pub use rule_outcomes::{ObjectVerdict, RuleOutcomes, RuleRecord, RuleVerdict, SelectorVerdict};
pub use section::{
    BodySection, EnvelopeRequest, EnvelopeSweep, Intrusion, Section, SectionAxis, SectionPolygon,
    SectionRequest,
};
pub use services::{ServiceRegistry, ServiceRegistryError, ServiceWatch};
pub use session::{
    EvidenceSession, EvidenceSessionError, SessionSources, SnapshotBoundService, SourceDisciplines,
    SourceSnapshot,
};
pub use setback::{
    SetbackArea, SetbackDirection, SetbackOther, SetbackPolygons, SetbackRequest, SetbackStrip,
};
pub use side_distance::{
    RectangleSide, SideDistance, SideDistanceRequest, SideDistances, SidePresence,
};
pub use sight::{
    SightError, SightEvidence, SightOutcome, SightRequest, SightService, SightServiceHandle,
    SkyObstruction, SkyObstructionRequest,
};
pub use sight_distance::{SightDistance, SightDistanceRequest, SightObstruction};
pub use source_metadata::{SourceMetadata, SourceMetadataIndex};
pub use space::{
    BoundaryGap, BoundaryRequest, Cap, CapCoverage, CapRequest, ClearHeightEvidence, Containment,
    OverlapRequest, SpaceAspect, SpaceError, SpaceOverlap, SpaceService, SpaceServiceHandle,
    SupportCounts, UnallocatedRegion, UnallocatedShare, UnmeasuredObjects,
};
pub use station_profile::{
    AlignmentProfile, AlignmentProfileRequest, ProfileExtreme, ProfileExtremeValue, ProfilePiece,
};
pub use table_files::{
    PackageDirectory, PackageFiles, TABLE_FILE_LIMIT_BYTES, TableFileError,
    load_definition_table_files, load_table_files, read_table_file,
};
pub use terrain::{
    TERRAIN_FILE_LIMIT_BYTES, TERRAIN_LOCATOR_PREFIX, TerrainElevation, TerrainError, TerrainFrame,
    TerrainKind, TerrainOutlineRequest, TerrainPointRequest, TerrainService, TerrainServiceHandle,
    TerrainSurface, TerrainSurfaceStatement, TinTerrain, terrain_kinds_read,
};
pub use topology::{
    CompleteTopologyEvidence, ConnectivityGraph, RouteOutcome, TopologyError, VerifiedConnection,
};
pub use triangle_count::{
    TriangleCount, TriangleCountError, TriangleCountService, TriangleCountServiceHandle,
};
pub use vertical_extent::{
    DirectionalExtent, EdgeSide, ElevationInterval, FaceEdge, FaceEdges, FaceNormal, FaceNormals,
    FacePiece, FacePieceSet, FacePieces, PointBox, SurfaceFace, Thickness, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
pub use walkability::{
    PassageAdmission, StretchLimit, VerifiedWalkablePassage, VerticalConnector,
    VerticalConnectorKind, WalkabilityError, WalkabilityRegion, WalkabilityRegionId,
    WalkabilityRequest, WalkabilityRouteOutcome, WalkabilityService, WalkabilityServiceHandle,
    WalkabilitySnapshot, WalkableStretch,
};
pub use walking_surface::{
    ClearWidthEvidence, ClearWidthRequest, ClearanceBelow, ClearanceBelowRequest, HandrailEvidence,
    HandrailRequest, Headroom, HeadroomRequest, Landing, LandingClearWidth,
    LandingClearWidthEvidence, LandingClearWidthRequest, LandingEvidence, LandingExtent,
    LandingRequest, MeasuredInterval, PlanSegment, RailMeasurement, RailSide, RiserClosure,
    SlopedRun, SlopedSurface, StretchPart, Tread, TreadFlight, TreadFlightRequest, WalkingEnd,
    WalkingLine, WalkingLinePlacement, WalkingStretch, WalkingSurfaceError, WalkingSurfaceService,
    WalkingSurfaceServiceHandle, across,
};
pub use zone_area::{PlanPolygon, ZoneArea, ZoneAreaRequest};

/// Binds a rule's outcomes to it, reporting each source-wide cause once.
///
/// An unbound concept depends on the package and the source, an unrecorded
/// fact on the source alone, and a missing service on the run, never on the
/// object, so every object of that source fails identically. Listing each
/// one buries the single cause under thousands of copies. Object-level
/// outcomes with any of these reasons
/// are merged per source, reason and message into one rule-level outcome
/// scoped to that source, naming the count and a few examples. Every other
/// outcome keeps its scope.
fn collapse_source_wide(
    rule_id: &RuleId,
    outcomes: Vec<CapabilityNotEvaluated>,
) -> Vec<NotEvaluated> {
    const EXAMPLES: usize = 3;
    let mut merged: BTreeMap<(axioval_ir::SourceId, NotEvaluatedReason, String), Vec<ObjectId>> =
        BTreeMap::new();
    let mut kept = Vec::new();
    for outcome in outcomes {
        match (outcome.reason, outcome.scope) {
            (
                reason @ (NotEvaluatedReason::UnboundConcept
                | NotEvaluatedReason::NotRecorded
                | NotEvaluatedReason::MissingService),
                Scope::Object(object),
            ) => merged
                .entry((object.source.clone(), reason, outcome.message))
                .or_default()
                .push(object),
            (reason, scope) => kept.push(NotEvaluated {
                explanation: outcome.explanation,
                rule_id: rule_id.clone(),
                scope,
                reason,
                message: outcome.message,
                location: outcome.location,
            }),
        }
    }
    kept.extend(merged.into_iter().map(|((source, reason, message), mut objects)| {
        objects.sort();
        let examples: Vec<&str> = objects
            .iter()
            .take(EXAMPLES)
            .map(|object| object.local_id.as_str())
            .collect();
        let more = objects.len().saturating_sub(EXAMPLES);
        let tail = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        NotEvaluated {
            explanation: None,
            rule_id: rule_id.clone(),
            scope: Scope::Source(source.clone()),
            reason,
            message: format!(
                "{message}; {} object(s) of source `{source}` not evaluated (e.g. {}{tail})",
                objects.len(),
                examples.join(", ")
            ),
            location: None,
        }
    }));
    kept
}

/// One rule's counts from its refined outcomes.
fn summarize(rule: &RuleId, checked: usize, evaluation: &CapabilityEvaluation) -> RuleSummary {
    let objects = |scopes: &mut dyn Iterator<Item = &Scope>| {
        scopes
            .filter_map(Scope::object)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    let failed = objects(&mut evaluation.findings.iter().map(|finding| &finding.scope));
    let open = objects(
        &mut evaluation
            .not_evaluated
            .iter()
            .map(|outcome| &outcome.scope),
    );
    RuleSummary::new(
        rule.clone(),
        checked,
        (failed, !evaluation.findings.is_empty()),
        (open, !evaluation.not_evaluated.is_empty()),
    )
}

/// Where a run is with one rule, told to a host's [`RuleObserver`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleProgress {
    /// The rule is about to run (its gate read, its capability evaluated).
    Started,
    /// The rule has run and its outcomes are refined.
    Finished,
}

/// A host's observer of each rule's progress ([`Runtime::with_rule_observer`]),
/// such as a timer. It sees rule ids only, never outcomes, and cannot change
/// a report.
pub type RuleObserver = Arc<dyn Fn(&RuleId, RuleProgress) + Send + Sync>;

/// A host's observer of how a run reused its derivations
/// ([`Runtime::with_derivation_observer`]): each kind's builds, hits,
/// waits, releases and bytes held at the end of the run. It never changes
/// a report.
pub type DerivationObserver = Arc<dyn Fn(&[DerivationStatistics]) + Send + Sync>;

/// Deterministic runtime that invokes only registered trusted capabilities.
pub struct Runtime {
    registry: CapabilityRegistry,
    services: ServiceRegistry,
    locations: Option<LocationPolicy>,
    summaries: bool,
    timings: bool,
    evaluation_budget: u64,
    parallelism: Parallelism,
    memory_budget: Option<Arc<MemoryBudget>>,
    observer: Option<RuleObserver>,
    derivations: Option<DerivationObserver>,
    terrain: Option<TerrainServiceHandle>,
    reuse: bool,
    keep: Option<u64>,
}
impl Runtime {
    /// Creates a runtime from a host-controlled registry.
    pub fn new(registry: CapabilityRegistry) -> Self {
        Self {
            registry,
            services: ServiceRegistry::new(),
            locations: None,
            summaries: false,
            timings: false,
            evaluation_budget: expression::DEFAULT_EVALUATION_BUDGET,
            parallelism: Parallelism::SERIAL,
            memory_budget: None,
            observer: None,
            derivations: None,
            terrain: None,
            reuse: true,
            keep: None,
        }
    }
    /// Installs what a run's expressions need: the plan's derived values
    /// and their types.
    fn install_expressions(
        services: &mut ServiceRegistry,
        derived: &Arc<BTreeMap<String, schema::Expression>>,
        types: &Arc<expression::DeclaredTypes>,
    ) {
        services.replace(values::ValueExpressions(derived.clone()));
        services.replace(types.clone());
    }
    /// How many expression nodes each rule may evaluate, its values and
    /// aggregate members included ([`expression::DEFAULT_EVALUATION_BUDGET`]
    /// unless set). Once its share is spent, every further expression of
    /// the rule leaves its object not evaluated (`resource_limit`), never
    /// passed.
    ///
    /// A share is the rule's own (#447): a value built once and read by
    /// several rules or subjects (a ruleset value, a measured value, a
    /// shared selection) is charged to every reader as if built for it
    /// ([`expression::budget`]), so what a rule is refused depends on its
    /// own request alone, never on the rules before it, the order they run
    /// in ([`ExecutionPlan::evaluate_in`]), the threads or what was reused
    /// ([`Self::without_reuse`]).
    ///
    /// The same number bounds the run: once the rules before a rule in
    /// plan order (and the classifications, groupings and relations
    /// evaluated before any rule) spent more than it, the rule is refused
    /// whole (`resource_limit`, one project-wide outcome) where it would
    /// spend any node or reads a rule refused so. A rule spending none is
    /// judged as ever.
    #[must_use]
    pub fn with_evaluation_budget(mut self, nodes: u64) -> Self {
        self.evaluation_budget = nodes;
        self
    }
    /// Evaluates each rule's subjects on up to `parallelism` threads
    /// ([`Parallelism`]); serial unless set. The report is the serial
    /// run's, byte for byte: a capability merges its subjects' outcomes in
    /// their order, never in the order threads finish them, and a panic is
    /// raised as the serial loop raises it.
    ///
    /// A rule's subjects spend one evaluation budget share whatever the
    /// threads ([`Self::with_evaluation_budget`]). Which subject a spent
    /// share refuses depends on the order they spend it in, so a rule whose
    /// share ran out on several threads is judged again on one, its report
    /// that serial judgement's; no other rule is judged again (#447). Each
    /// thread evaluates one subject at a time,
    /// with what that subject's measurement holds; the heavy steps of
    /// those measurements are bounded together by the memory budget
    /// ([`Self::with_memory_budget`]), not by the threads.
    #[must_use]
    pub fn with_parallelism(mut self, parallelism: Parallelism) -> Self {
        self.parallelism = parallelism;
        self
    }
    /// Admits the heavy steps of each run against `budget` ([`memory`]):
    /// a step that would pass it beside the steps admitted already waits
    /// for one of them to finish, so the memory they take together does
    /// not grow with the threads. Several runtimes, or several runs at
    /// once, may share one budget. Without one, steps are admitted at
    /// once.
    ///
    /// The report is the one without a budget, byte for byte, but where
    /// a step's own estimate is more than the whole budget: that step is
    /// refused by name, whatever else is running, and what needed it is
    /// not evaluated.
    #[must_use]
    pub fn with_memory_budget(mut self, budget: Arc<MemoryBudget>) -> Self {
        self.memory_budget = Some(budget);
        self
    }
    /// Reports per-rule counts and status ([`Report::rules`]): how many
    /// objects each rule surely selected, how many it found or left not
    /// evaluated, and whether it passed, failed, was not evaluated or
    /// selected nothing. Off by default, and then reports are unchanged.
    /// Needs the registry's outcome refiner to count selections.
    #[must_use]
    pub fn with_rule_summaries(mut self) -> Self {
        self.summaries = true;
        self
    }
    /// Reports how long each rule took: every rule summary that ran carries
    /// a [`RuleTiming`](axioval_ir::RuleTiming), the wall time spent
    /// evaluating the rule and the part of it spent in the host services it
    /// called (geometry measurement, proximity, routes and the like), per
    /// service. The object count is the summary's `checked`. Implies
    /// [`Self::with_rule_summaries`]; a rule its gate skipped is timed too,
    /// one the plan deferred is not.
    ///
    /// Timings are diagnostics: they vary between machines and runs, never
    /// change a finding, and summaries compare equal with or without them.
    /// Off by default, and then the engine reads no clock.
    #[must_use]
    pub fn with_rule_timings(mut self) -> Self {
        self.summaries = true;
        self.timings = true;
        self
    }
    /// Locates every finding and not-evaluated outcome by storey and space
    /// as `policy` says. Off by default, and then reports carry no
    /// location. Needs the registry's outcome refiner.
    #[must_use]
    pub fn with_locations(mut self, policy: LocationPolicy) -> Self {
        self.locations = Some(policy);
        self
    }
    /// Tells `observer` when each rule of the plan starts and finishes, in
    /// plan order, so a host can time rules. The engine reads no clock; the
    /// observer never changes the report.
    #[must_use]
    pub fn with_rule_observer(mut self, observer: RuleObserver) -> Self {
        self.observer = Some(observer);
        self
    }
    /// Serves `terrain` to every run as context beside the model: the
    /// ground surfaces heights above the terrain are measured against. It
    /// replaces any terrain a host registered among a session's services,
    /// so a run measures against the one the runtime was given. Without
    /// one, every value reading the terrain is not evaluated.
    #[must_use]
    pub fn with_terrain(mut self, terrain: TerrainServiceHandle) -> Self {
        self.terrain = Some(terrain);
        self
    }
    /// Keeps nothing a run builds for reuse: every run's derivation store
    /// is forgetful ([`DerivationStore::forgetful`]), so each derivation,
    /// measurement and shared selection is built every time it is read.
    /// The report is the same byte for byte, only slower: every value is a
    /// pure function of its key, and a budget charges a reused value as if
    /// built (ADR 0006). Only to show that (the determinism gate, #441);
    /// services a host built keep their own caches unless it turns them
    /// off too.
    #[must_use]
    pub fn without_reuse(mut self) -> Self {
        self.reuse = false;
        self
    }
    /// Keeps at most `bytes` of what a run's derivation store builds, all
    /// kinds together ([`DerivationStore::keeping`]): past it, the values
    /// read least recently are released and built again when read again.
    /// The report is the same byte for byte, only slower, however small
    /// the bound (#440): every value is a pure function of its key, and a
    /// budget charges a reused value as if built. Without it, a store is
    /// bounded by its kinds' own bounds and the check's memory budget, and
    /// releases each value after the last rule that may read it.
    #[must_use]
    pub fn with_kept_derivations(mut self, bytes: u64) -> Self {
        self.keep = Some(bytes);
        self
    }
    /// Tells `observer`, at the end of each run, how its derivation store
    /// was reused ([`DerivationStore::statistics`]), such as for a host's
    /// timings.
    #[must_use]
    pub fn with_derivation_observer(mut self, observer: DerivationObserver) -> Self {
        self.derivations = Some(observer);
        self
    }
    /// Adds adapter-provided host services to subsequent evaluations.
    #[must_use]
    pub fn with_services(mut self, services: ServiceRegistry) -> Self {
        self.services = services;
        self
    }
    /// Executes a plan and returns deterministically sorted findings.
    ///
    /// Execution fails closed if the host registry no longer contains any capability
    /// that was present when the plan was compiled.
    ///
    /// A bare project carries no source type-system declarations, so package
    /// concepts bind to nothing and concept-based selection is reported as not
    /// evaluated. Hosts that want concept binding run an [`EvidenceSession`].
    pub fn run(&self, project: &Project, plan: ExecutionPlan) -> Result<Report, EngineError> {
        self.run_project(project, plan).map(|(report, _)| report)
    }

    /// Executes a plan as [`Self::run`] does, and returns beside the report
    /// every rule's [`RuleRecord`], each with its applicability selection
    /// recorded: how the rule judged every object, passed and not selected
    /// told apart. The selection is recorded through the registry's
    /// [`OutcomeRefiner`], so a registry without one is refused.
    ///
    /// The report is the one [`Self::run`] returns; recording costs one more
    /// selector evaluation per rule and object. The differential parity
    /// harness reads it.
    ///
    /// # Errors
    ///
    /// As [`Self::run`], and [`EngineError::MissingRefiner`] without an
    /// outcome refiner.
    pub fn run_recorded(
        &self,
        project: &Project,
        plan: ExecutionPlan,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        self.run_project(project, self.recording(plan)?)
    }

    /// Executes a plan against one session as [`Self::run_session`] does,
    /// and returns every rule's record as [`Self::run_recorded`] does.
    ///
    /// # Errors
    ///
    /// As [`Self::run_session`], and [`EngineError::MissingRefiner`] without
    /// an outcome refiner.
    pub fn run_session_recorded(
        &self,
        session: &EvidenceSession,
        plan: ExecutionPlan,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        self.run_in_session(session, self.recording(plan)?)
    }

    /// `plan` with every rule's selection recorded.
    fn recording(&self, mut plan: ExecutionPlan) -> Result<ExecutionPlan, EngineError> {
        if self.registry.refiner().is_none() {
            return Err(EngineError::MissingRefiner(
                "recording each rule's selection",
            ));
        }
        let every: Vec<RuleId> = plan.rules.iter().map(|rule| rule.id.clone()).collect();
        plan.recorded.extend(every);
        Ok(plan)
    }

    fn run_project(
        &self,
        project: &Project,
        plan: ExecutionPlan,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        self.run_with_services(
            project,
            &self.services,
            BTreeMap::new(),
            (
                SessionSources::new(project.objects().map(|object| object.id.source.clone())),
                SourceDisciplines::default(),
                SourceMetadataIndex::default(),
            ),
            plan,
        )
    }

    /// Executes a plan against one immutable source/evidence snapshot.
    pub fn run_session(
        &self,
        session: &EvidenceSession,
        plan: ExecutionPlan,
    ) -> Result<Report, EngineError> {
        self.run_in_session(session, plan).map(|(report, _)| report)
    }

    fn run_in_session(
        &self,
        session: &EvidenceSession,
        plan: ExecutionPlan,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        let type_systems = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), snapshot.type_systems().to_vec()))
            .collect();
        // A superseded object is evaluated only through the object
        // superseding it: the rules see a project without it (#390).
        let evaluated = evaluated_project(session);
        let (mut report, outcomes) = self.run_with_services(
            evaluated.as_ref().unwrap_or_else(|| session.project()),
            session.services(),
            type_systems,
            (
                SessionSources::new(
                    session
                        .snapshots()
                        .map(|snapshot| snapshot.source().clone()),
                ),
                session.source_disciplines(),
                session.metadata_index(),
            ),
            plan,
        )?;
        if evaluated.is_some() {
            report.superseded = session.superseded().cloned().collect();
            redirect_superseded(&mut report);
        }
        Ok((report, outcomes))
    }

    /// The registry's outcome refiner, refused missing when the plan or
    /// the host's options need one.
    fn refiner_for(
        &self,
        plan: &ExecutionPlan,
    ) -> Result<Option<&Arc<dyn OutcomeRefiner>>, EngineError> {
        let refiner = self.registry.refiner();
        if refiner.is_some() {
            return Ok(refiner);
        }
        if !plan.recorded.is_empty() {
            return Err(EngineError::MissingRefiner(
                "reading another rule's outcomes per object",
            ));
        }
        if !plan.classifications.is_empty() {
            return Err(EngineError::MissingRefiner("deriving classifications"));
        }
        if !plan.groupings.is_empty() {
            return Err(EngineError::MissingRefiner("deriving groups"));
        }
        if !plan.relations.is_empty() {
            return Err(EngineError::MissingRefiner("deriving relations"));
        }
        if self.locations.is_some() {
            return Err(EngineError::MissingRefiner("locating outcomes"));
        }
        if self.summaries {
            return Err(EngineError::MissingRefiner(
                "counting each rule's selection",
            ));
        }
        Ok(None)
    }

    /// Evaluates `rule`, then grades and refines its outcomes.
    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        capability: &dyn RuleCapability,
        rule: &CompiledRule,
        refinement: Option<&RuleRefinement>,
    ) -> CapabilityEvaluation {
        let mut evaluation = capability.evaluate(context, rule);
        // What only this rule read is dropped with it.
        if let Some(memo) = context.services.get::<MeasuredMemo>() {
            memo.end_rule();
        }
        if let Some(refinement) = refinement {
            evaluation.grade(&refinement.severity_bands);
        }
        if let Some(refiner) = self.registry.refiner()
            && (refinement.is_some() || self.locations.is_some())
        {
            let unrefined = RuleRefinement::default();
            let refining = Refining {
                refinement: refinement.unwrap_or(&unrefined),
                locations: self.locations.as_ref(),
            };
            refiner.refine(context, rule, &refining, &mut evaluation);
        }
        evaluation
    }

    /// The deferred rules' project-scoped outcomes, and their summaries
    /// when the host asked for them.
    fn deferred(
        &self,
        deferred: Vec<DeferredRule>,
        summaries: &mut Vec<RuleSummary>,
    ) -> Vec<NotEvaluated> {
        deferred
            .into_iter()
            .inspect(|rule| {
                if self.summaries {
                    summaries.push(RuleSummary::new(rule.id.clone(), 0, (0, false), (0, true)));
                }
            })
            .map(|rule| NotEvaluated {
                explanation: None,
                rule_id: rule.id,
                scope: Scope::Project,
                reason: NotEvaluatedReason::InvalidDeclaration,
                message: rule.reason,
                location: None,
            })
            .collect()
    }

    /// Runs `plan` at the runtime's parallelism.
    fn run_with_services(
        &self,
        project: &Project,
        services: &ServiceRegistry,
        type_systems: BTreeMap<axioval_ir::SourceId, Vec<Arc<str>>>,
        session: (SessionSources, SourceDisciplines, SourceMetadataIndex),
        plan: ExecutionPlan,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        // The run's steps are admitted against its budget, on this thread
        // and on every worker it fans out to; without one, against any
        // budget the host entered on this thread.
        let _budget = self.memory_budget.as_ref().map(MemoryBudget::enter);
        self.run_at(
            project,
            services,
            (type_systems, session),
            plan,
            self.parallelism,
        )
    }

    /// Runs `plan` with each rule's subjects evaluated on `parallelism`
    /// threads.
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn run_at(
        &self,
        project: &Project,
        services: &ServiceRegistry,
        (type_systems, (sources, disciplines, metadata)): (
            BTreeMap<axioval_ir::SourceId, Vec<Arc<str>>>,
            (SessionSources, SourceDisciplines, SourceMetadataIndex),
        ),
        plan: ExecutionPlan,
        parallelism: Parallelism,
    ) -> Result<(Report, RuleOutcomes), EngineError> {
        // Bindings are per run: they join this plan's package concepts to this
        // project's declared type systems, so they cannot be registered once by
        // a host. A host-registered `ConceptBindings` is overridden rather than
        // trusted, because it could bind concepts the packages never declared.
        let mut services = services.clone();
        services.replace(ConceptBindings::new(plan.concepts.clone(), type_systems));
        // Disciplines are the session's declarations; a host-registered copy
        // could claim roles the session never declared.
        services.replace(disciplines);
        // So is what the session knows about each source as a whole.
        services.replace(metadata);
        // So are the sources: a host copy could hide an empty source.
        services.replace(sources);
        // So is the number of threads: the runtime's, whatever a host
        // registered.
        services.replace(parallelism);
        // The terrain is context the host gave the runtime.
        if let Some(terrain) = &self.terrain {
            services.replace(terrain.clone());
        }
        // And the resource objects the rules name: a host copy could claim
        // resource objects the session never listed.
        resources::install(&mut services, project, &plan.rules);
        // The flow through the sources' distribution networks, a derived
        // relationship every path reads like a stated one.
        network::install(&mut services);
        let refiner = self.refiner_for(&plan)?;
        // Each derivation is released after the last rule that may read
        // it (#440).
        let readers = LastReaders::of(&geometry_demand::services_asked(&plan, &self.registry));
        let mut summaries: Vec<RuleSummary> = Vec::new();
        let mut findings = Vec::new();
        let mut tables: Vec<ReportTable> = Vec::new();
        let mut not_evaluated = self.deferred(plan.deferred, &mut summaries);
        Self::install_expressions(&mut services, &plan.values, &plan.types);
        // The catalogues are the plan's: a host copy could state entries it
        // never supplied through the plan.
        services.replace(Arc::new(plan.catalogues.clone()));
        // The context zones are the plan's: a host copy could state zones
        // it never supplied through the plan.
        services.replace(plan.zones.clone());
        // One derivation store per check: nothing built in one run answers
        // another, whatever a host registered.
        let store = match (self.reuse, self.keep) {
            (false, _) => DerivationStore::forgetful(),
            (true, Some(bytes)) => DerivationStore::keeping(bytes),
            (true, None) => DerivationStore::new(),
        };
        services.replace(store.clone());
        // What the store keeps gives way to a step needing the room under
        // the check's memory budget (#440).
        if let Some(budget) = memory::current() {
            budget.reclaim_from(store.reclaimer());
        }
        // A geometry adapter's service methods read no registry, so they
        // reach the store as the check's current one, on this thread and on
        // every worker it fans out to (`derivation::current`).
        let _store = store.enter();
        // Expressions evaluated before any rule (classifications, groupings
        // and relations reading values) spend a budget of the run's size,
        // counted first by the backstop (#447).
        let before_rules = Arc::new(expression::EvaluationBudget::new(self.evaluation_budget));
        let _before_rules = before_rules.enter();
        self.registry.install_measured(&mut services, project);
        // Measured values are answered through the host's resolver in
        // every run; classifications are derived first when the plan has
        // any.
        derive_before_rules(
            refiner.map(AsRef::as_ref),
            project,
            &mut services,
            (
                &plan.classifications,
                &plan.groupings,
                (&plan.relations, &plan.supplied),
            ),
        );
        // What every completed rule reported, for the rules that read it.
        let mut outcomes = RuleOutcomes::default();
        let mut backstop = Backstop::new(
            self.evaluation_budget,
            plan.planned.as_deref(),
            &plan.rules,
            before_rules.spent(),
        );
        let mut ran: Vec<RuleOutput> = Vec::new();
        for (index, rule) in plan.rules.iter().enumerate() {
            store.release_unread(readers.unread_from(index));
            let capability = self
                .registry
                .get(&rule.capability)
                .ok_or_else(|| EngineError::UnknownCapability(rule.capability.clone()))?;
            let rule_id = rule.id.clone();
            let _progress = self
                .observer
                .as_ref()
                .map(|observer| Observed::start(observer, &rule_id));
            // An auxiliary rule's outcome is recorded for the rules that
            // read it, and reported only through them.
            let reported = !plan.auxiliary.contains(&rule_id);
            let reads = plan.dependencies.get(&rule_id);
            // Past the backstop, a rule reading a refused one is refused
            // unrun, and any other is given nothing to spend.
            let past = backstop.past(&rule_id);
            if past == Some(true) && backstop.reads_refused(reads) {
                backstop.done(&rule_id, 0, true, reads);
                let refusal = backstop.refusal();
                outcomes.insert(rule_id.clone(), RuleRecord::of(&refusal, None));
                ran.push(RuleOutput::refused(
                    rule_id,
                    reported,
                    refusal,
                    self.summaries,
                ));
                continue;
            }
            let share = if past == Some(true) {
                0
            } else {
                self.evaluation_budget
            };
            let mut budget = Arc::new(expression::EvaluationBudget::within(
                share,
                self.evaluation_budget,
            ));
            let mut entered = budget.enter();
            // A rule reads the outcomes of the rules the plan ran before it.
            services.replace(outcomes.clone());
            let clock = self.timings.then(timing::RuleClock::start);
            let context = RuleContext {
                project,
                services: &services,
            };
            let gates = plan.gates.get(&rule_id).map_or(&[][..], Vec::as_slice);
            let evaluation = match outcomes.gate(gates) {
                rule_outcomes::Gate::Closed => {
                    outcomes.insert(rule_id.clone(), RuleRecord::skipped());
                    backstop.done(&rule_id, 0, false, reads);
                    let summary = (self.summaries && reported)
                        .then(|| timed(RuleSummary::skipped(rule_id.clone()), clock));
                    ran.push(RuleOutput::of(
                        rule_id,
                        reported,
                        past.is_none(),
                        CapabilityEvaluation::default(),
                        summary,
                    ));
                    continue;
                }
                rule_outcomes::Gate::Undecided(why) => {
                    outcomes.insert(rule_id.clone(), RuleRecord::undecided(&why));
                    CapabilityEvaluation::not_evaluated(NotEvaluatedReason::IncompleteEvidence, why)
                }
                rule_outcomes::Gate::Open => {
                    let judge = |context: &RuleContext<'_>| {
                        let evaluation = self.evaluate(
                            context,
                            capability.as_ref(),
                            rule,
                            plan.refinements.get(&rule_id),
                        );
                        let selection =
                            refiner
                                .filter(|_| plan.recorded.contains(&rule_id))
                                .map(|refiner| {
                                    rule_outcomes::selection(refiner.as_ref(), context, rule)
                                });
                        (evaluation, selection)
                    };
                    let (mut evaluation, mut selection) = judge(&context);
                    // Which subject a spent share refuses depends on the
                    // order the threads spend it in: the rule is judged
                    // again on one thread, its subjects in their order. A
                    // rule refused by the backstop is not judged again.
                    if budget.refused() && !parallelism.is_serial() && past != Some(true) {
                        drop((evaluation, selection));
                        drop(entered);
                        budget = Arc::new(expression::EvaluationBudget::within(
                            share,
                            self.evaluation_budget,
                        ));
                        entered = budget.enter();
                        let mut serial = services.clone();
                        serial.replace(Parallelism::SERIAL);
                        (evaluation, selection) = judge(&RuleContext {
                            project,
                            services: &serial,
                        });
                    }
                    outcomes.insert(rule_id.clone(), RuleRecord::of(&evaluation, selection));
                    evaluation
                }
            };
            // What the rule spent, recording its selection for the rules
            // reading it included; its summary's count below is not.
            let tried = budget.spent() > 0 || budget.refused();
            backstop.done(&rule_id, budget.spent(), tried, reads);
            if past == Some(true) && tried {
                let refusal = backstop.refusal();
                outcomes.insert(rule_id.clone(), RuleRecord::of(&refusal, None));
                ran.push(RuleOutput::refused(
                    rule_id,
                    reported,
                    refusal,
                    self.summaries,
                ));
                continue;
            }
            // The time is the rule's own: counting its selection for the
            // summary below is the host's diagnostic, not the rule's work.
            let timing = clock.map(timing::RuleClock::finish);
            if !reported {
                ran.push(RuleOutput::of(
                    rule_id,
                    reported,
                    past.is_none(),
                    CapabilityEvaluation::default(),
                    None,
                ));
                continue;
            }
            let summary = refiner.filter(|_| self.summaries).map(|refiner| {
                let summary = summarize(&rule_id, refiner.selected(&context, rule), &evaluation);
                match timing {
                    Some(timing) => summary.with_timing(timing),
                    None => summary,
                }
            });
            drop(entered);
            ran.push(RuleOutput::of(
                rule_id,
                reported,
                past.is_none(),
                evaluation,
                summary,
            ));
        }
        // A rule run before the backstop knew of it is refused now where
        // the backstop refuses it in plan order.
        for output in &mut ran {
            if output.speculative && backstop.refuses(&output.rule) {
                let refusal = backstop.refusal();
                outcomes.insert(output.rule.clone(), RuleRecord::of(&refusal, None));
                *output = RuleOutput::refused(
                    output.rule.clone(),
                    output.reported,
                    refusal,
                    self.summaries,
                );
            }
        }
        for output in ran {
            if !output.reported {
                continue;
            }
            summaries.extend(output.summary);
            findings.extend(output.findings);
            tables.extend(output.tables);
            not_evaluated.extend(output.not_evaluated);
        }
        store.release_unread(readers.unread_from(usize::MAX));
        if let Some(observer) = &self.derivations {
            observer(&store.statistics());
        }
        Ok((
            assemble(findings, not_evaluated, tables, summaries, &services)?,
            outcomes,
        ))
    }
}

/// The last rule (by its index in the plan) that may ask each host
/// service, so the run releases the derivations read through a service no
/// rule still to run asks (#440).
struct LastReaders {
    /// The last rule that may ask each service it names.
    named: BTreeMap<TypeId, usize>,
    /// The last rule that may ask any service.
    any: Option<usize>,
}

impl LastReaders {
    fn of(asked: &[Option<BTreeSet<TypeId>>]) -> Self {
        let mut named = BTreeMap::new();
        let mut any = None;
        for (index, services) in asked.iter().enumerate() {
            match services {
                Some(services) => {
                    for service in services {
                        named.insert(*service, index);
                    }
                }
                None => any = Some(index),
            }
        }
        Self { named, any }
    }

    /// Whether no rule from the `index`th on asks a service.
    fn unread_from(&self, index: usize) -> impl Fn(TypeId) -> bool + '_ {
        move |service| {
            self.any.is_none_or(|last| last < index)
                && self.named.get(&service).is_none_or(|last| *last < index)
        }
    }
}

/// One rule's reported outcomes, kept until the backstop has decided
/// whether it refuses the rule.
struct RuleOutput {
    rule: RuleId,
    reported: bool,
    /// Whether it ran before every rule ahead of it in plan order had.
    speculative: bool,
    findings: Vec<Finding>,
    tables: Vec<ReportTable>,
    not_evaluated: Vec<NotEvaluated>,
    summary: Option<RuleSummary>,
}

impl RuleOutput {
    /// What `rule` reported in `evaluation`.
    fn of(
        rule: RuleId,
        reported: bool,
        speculative: bool,
        mut evaluation: CapabilityEvaluation,
        summary: Option<RuleSummary>,
    ) -> Self {
        Self {
            findings: std::mem::take(&mut evaluation.findings),
            // The compiled rule is the table's identity, whatever the
            // capability named.
            tables: std::mem::take(&mut evaluation.tables)
                .into_iter()
                .map(|table| table.with_rule_id(rule.clone()))
                .collect(),
            not_evaluated: collapse_source_wide(&rule, evaluation.not_evaluated),
            rule,
            reported,
            speculative,
            summary,
        }
    }

    /// A rule the backstop refused, as `refusal` reports it, summarized
    /// where `summaries`.
    fn refused(
        rule: RuleId,
        reported: bool,
        refusal: CapabilityEvaluation,
        summaries: bool,
    ) -> Self {
        let summary = summaries.then(|| RuleSummary::new(rule.clone(), 0, (0, false), (0, true)));
        Self::of(rule, reported, false, refusal, summary)
    }
}

/// The run-wide backstop of the expression evaluation budget (#447): once
/// the rules before a rule in plan order (and what was evaluated before any
/// rule) spent more than the run's size, the rule is refused whole where
/// it would spend anything, or reads a rule refused so. Each rule's
/// spending is its own (its share, charged as if built), so which rules
/// the backstop refuses depends on the plan alone, never on the order the
/// rules ran in: a rule that ran before the rules ahead of it is refused
/// once they have.
struct Backstop {
    limit: u64,
    /// Each rule's place in plan order.
    ranks: BTreeMap<RuleId, usize>,
    /// What each rule in plan order spent and whether it tried to spend
    /// anything, once it ran.
    spent: Vec<Option<(u64, bool, Vec<RuleId>)>>,
    /// The rules in plan order that ran, from the first.
    known: usize,
    /// What was spent before the rules, and by the first `known` rules.
    total: u64,
    /// The rules refused, in plan order up to `known`.
    refused: BTreeSet<RuleId>,
    planned: Vec<RuleId>,
}

impl Backstop {
    fn new(
        limit: u64,
        planned: Option<&[RuleId]>,
        rules: &[CompiledRule],
        before_rules: u64,
    ) -> Self {
        let planned: Vec<RuleId> = planned.map_or_else(
            || rules.iter().map(|rule| rule.id.clone()).collect(),
            <[RuleId]>::to_vec,
        );
        Self {
            limit,
            ranks: planned
                .iter()
                .enumerate()
                .map(|(rank, rule)| (rule.clone(), rank))
                .collect(),
            spent: std::iter::repeat_with(|| None)
                .take(planned.len())
                .collect(),
            known: 0,
            total: before_rules,
            refused: BTreeSet::new(),
            planned,
        }
    }

    /// Whether `rule` is past the backstop: `None` while a rule before it
    /// in plan order is still to run.
    fn past(&self, rule: &RuleId) -> Option<bool> {
        let rank = *self.ranks.get(rule)?;
        (self.known >= rank).then_some(self.total > self.limit)
    }

    /// Whether any of `reads` was refused.
    fn reads_refused(&self, reads: Option<&BTreeSet<RuleId>>) -> bool {
        reads.is_some_and(|reads| reads.iter().any(|read| self.refused.contains(read)))
    }

    /// Records that `rule` spent `spent` (`tried`: anything at all) and
    /// reads `reads`.
    fn done(&mut self, rule: &RuleId, spent: u64, tried: bool, reads: Option<&BTreeSet<RuleId>>) {
        let Some(&rank) = self.ranks.get(rule) else {
            return;
        };
        let reads = reads.map_or_else(Vec::new, |reads| reads.iter().cloned().collect());
        self.spent[rank] = Some((spent, tried, reads));
        // Every rule a rule reads is before it in plan order, so whether
        // it reads a refused one is known once the rules before it ran.
        while let Some(Some((spent, tried, reads))) = self.spent.get(self.known) {
            let refused = self.total > self.limit
                && (*tried || reads.iter().any(|read| self.refused.contains(read)));
            if refused {
                self.refused.insert(self.planned[self.known].clone());
            } else {
                self.total = self.total.saturating_add(*spent);
            }
            self.known += 1;
        }
    }

    /// Whether the backstop refuses `rule`, once every rule ran.
    fn refuses(&self, rule: &RuleId) -> bool {
        self.refused.contains(rule)
    }

    /// What a refused rule reports: one project-wide outcome.
    fn refusal(&self) -> CapabilityEvaluation {
        CapabilityEvaluation::not_evaluated(
            NotEvaluatedReason::ResourceLimit,
            format!(
                "the rules before it spent the run's expression evaluation budget of {} \
                 nodes",
                self.limit
            ),
        )
    }
}

/// The session's project less its superseded objects, `None` when it
/// declares none (#390).
fn evaluated_project(session: &EvidenceSession) -> Option<Project> {
    if session.superseded().len() == 0 {
        return None;
    }
    let superseded: BTreeSet<&ObjectId> = session.superseded().map(|entry| &entry.object).collect();
    Some(session.project().without(&superseded))
}

/// Names the superseding object wherever an outcome still names a
/// superseded one (#390).
///
/// Rules select subjects and counterparts from the evaluated project, so
/// none names a superseded object; a host service searching every body it
/// holds (a guard's barriers, a space's bounding elements when a rule
/// names none) may still find one. It is the same body as the object
/// superseding it, which is named instead, once.
fn redirect_superseded(report: &mut Report) {
    let by: BTreeMap<&ObjectId, &ObjectId> = report
        .superseded
        .iter()
        .map(|entry| (&entry.object, &entry.by))
        .collect();
    let redirect = |scope: &mut Scope| {
        if let Scope::Object(object) = scope
            && let Some(superseding) = by.get(object)
        {
            *object = (*superseding).clone();
        }
    };
    for finding in &mut report.findings {
        redirect(&mut finding.scope);
        if finding.related.iter().any(|object| by.contains_key(object)) {
            let mut related = Vec::with_capacity(finding.related.len());
            for object in &finding.related {
                let object = by.get(object).map_or(object, |superseding| *superseding);
                if !related.contains(object) && finding.scope.object() != Some(object) {
                    related.push(object.clone());
                }
            }
            finding.related = related;
        }
    }
    for outcome in &mut report.not_evaluated {
        redirect(&mut outcome.scope);
    }
}

/// `summary` with the time its clock measured, if one ran.
fn timed(summary: RuleSummary, clock: Option<timing::RuleClock>) -> RuleSummary {
    match clock {
        Some(clock) => summary.with_timing(clock.finish()),
        None => summary,
    }
}

/// A rule a [`RuleObserver`] was told started; it is told the rule
/// finished when this is dropped, on every path out of the rule.
struct Observed<'a> {
    observer: &'a RuleObserver,
    rule: RuleId,
}

impl<'a> Observed<'a> {
    fn start(observer: &'a RuleObserver, rule: &RuleId) -> Self {
        observer(rule, RuleProgress::Started);
        Self {
            observer,
            rule: rule.clone(),
        }
    }
}

impl Drop for Observed<'_> {
    fn drop(&mut self) {
        (self.observer)(&self.rule, RuleProgress::Finished);
    }
}

/// The resource objects the outcomes name, sorted by identity.
fn named_resources(
    findings: &[Finding],
    not_evaluated: &[NotEvaluated],
    tables: &[ReportTable],
    services: &ServiceRegistry,
) -> Vec<axioval_ir::Object> {
    let Some(resources) = services
        .get::<ResourceObjects>()
        .filter(|resources| !resources.is_empty())
    else {
        return Vec::new();
    };
    let mut named: BTreeSet<&ObjectId> = BTreeSet::new();
    for finding in findings {
        named.extend(finding.object_id());
        named.extend(&finding.related);
    }
    named.extend(not_evaluated.iter().filter_map(NotEvaluated::object_id));
    named.extend(
        tables
            .iter()
            .flat_map(|table| table.rows().iter().filter_map(|row| row.scope().object())),
    );
    named
        .into_iter()
        .filter_map(|id| resources.object(id).cloned())
        .collect()
}

/// The plan's relations and the pairs the host supplied, by relation id.
type RelationInputs<'a> = (
    &'a [schema::RelationDefinition],
    &'a BTreeMap<String, SuppliedPairs>,
);

/// Installs the run's derived properties over the host's resolver: the
/// classes of the plan's classifications, then its groups, derived after
/// the classes their members and keys may read and installed beside the
/// host's services.
fn derive_before_rules(
    refiner: Option<&dyn OutcomeRefiner>,
    project: &Project,
    services: &mut ServiceRegistry,
    (classifications, groupings, relations): (
        &[schema::ClassificationDefinition],
        &[schema::GroupingDefinition],
        RelationInputs<'_>,
    ),
) {
    let host = services.get::<PropertyResolutionServiceHandle>().cloned();
    let mut classes = Arc::<Classifications>::default();
    if classifications.is_empty()
        && let Some(host) = &host
    {
        derived::install(
            services,
            Some(host),
            Arc::default(),
            Arc::default(),
            project,
        );
    }
    if let Some(refiner) = refiner.filter(|_| !classifications.is_empty()) {
        classes = derive_classifications(refiner, project, services, classifications);
    }
    if let Some(refiner) = refiner.filter(|_| !groupings.is_empty()) {
        let groups = Arc::new(groupings::derive_all(refiner, project, services, groupings));
        groupings::install(services, &groups);
        derived::install(services, host.as_ref(), classes, groups, project);
    }
    // Relations read classes and groups, never one another.
    if let Some(refiner) = refiner.filter(|_| !relations.0.is_empty()) {
        let relations = Arc::new(relations::derive_all(refiner, project, services, relations));
        relations::install(services, &relations);
    }
}

/// Classifies every object by each of `definitions`, in order, and installs
/// the run's property resolver answering the classification set: each
/// classification's rows read the ones derived before it.
fn derive_classifications(
    refiner: &dyn OutcomeRefiner,
    project: &Project,
    services: &mut ServiceRegistry,
    definitions: &[schema::ClassificationDefinition],
) -> Arc<Classifications> {
    let host = services.get::<PropertyResolutionServiceHandle>().cloned();
    let mut derived = Classifications::default();
    for definition in definitions {
        derived::install(
            services,
            host.as_ref(),
            Arc::new(derived.clone()),
            Arc::default(),
            project,
        );
        let context = RuleContext { project, services };
        derived.classify(refiner, &context, definition);
    }
    let derived = Arc::new(derived);
    derived::install(
        services,
        host.as_ref(),
        derived.clone(),
        Arc::default(),
        project,
    );
    derived
}

/// The report of every rule's outcomes, each part in its deterministic
/// order, with the resource objects of `services` they name; a rule
/// reporting one table name twice fails the run.
fn assemble(
    mut findings: Vec<Finding>,
    mut not_evaluated: Vec<NotEvaluated>,
    mut tables: Vec<ReportTable>,
    mut summaries: Vec<RuleSummary>,
    services: &ServiceRegistry,
) -> Result<Report, EngineError> {
    findings.sort_by(|a, b| {
        a.rule_id
            .cmp(&b.rule_id)
            // Project, then sources, then objects, each by identity.
            .then_with(|| a.scope.cmp(&b.scope))
            .then_with(|| a.message.cmp(&b.message))
    });
    not_evaluated.sort();
    tables.sort_by(|a, b| (a.rule_id(), a.name()).cmp(&(b.rule_id(), b.name())));
    if let Some(pair) = tables
        .windows(2)
        .find(|pair| (pair[0].rule_id(), pair[0].name()) == (pair[1].rule_id(), pair[1].name()))
    {
        return Err(EngineError::DuplicateReportTable {
            rule: pair[0].rule_id().to_string(),
            table: pair[0].name().to_owned(),
        });
    }
    summaries.sort();
    Ok(Report {
        resources: named_resources(&findings, &not_evaluated, &tables, services),
        stale_decisions: Vec::new(),
        superseded: Vec::new(),
        findings,
        not_evaluated,
        tables,
        rules: summaries,
    })
}
