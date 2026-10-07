# Capability templates

A built-in capability is a named, parameterized rule type: packages, the
[authoring catalogue](./catalogue.md) and the [export profiles](./export.md)
address it as one rule, and external rule formats know many of them as one
rule type each. Inside, a capability is being rebuilt from the shared parts
of [composed rules](./composing-rules.md): measured values, expressions,
aggregates, generic judges and, where needed, a search. A **template** is
that rebuild: the capability's outside contract, kept unchanged, and a
composition the engine evaluates with the same evaluator as any
`expression` rule, so an improvement to a part improves every template
that uses it.

Templates are engine-owned and built in. A package never supplies one,
and nothing in a template runs package code: it is data the engine reads,
like an expression.

`body-extent` is the first capability that runs as a template
([#278](https://github.com/axioval/engine/issues/278)); `triangle-count`
and `plan-area` follow ([#282](https://github.com/axioval/engine/issues/282)),
and `property-predicate` is the first of the generic judges
([#287](https://github.com/axioval/engine/issues/287)); `object-count`
is the first deciding per source or for the project
([#290](https://github.com/axioval/engine/issues/290)); `shelf-capacity` is the
first whose values take the rule's selectors
([#289](https://github.com/axioval/engine/issues/289)); `area-ratio` is the
first over two populations and a derived ratio
([#290](https://github.com/axioval/engine/issues/290)); `consistent-value`
and `selector-conformance` follow `unique-value` as group decisions, and
`relative-count` judges two populations by a proportion, and
`property-value` judges XML Schema facets, `property-requirements`
requirements tables and `property-comparison` candidates against a target
(#287): every generic judge now runs as a template; `ramp-geometry` is the
first judging the items of measured lists one by one, and
`stair-geometry` the first judging the parts of what it selects as objects
of their own ([#280](https://github.com/axioval/engine/issues/280)); `slab-contact` is the first to grade its findings itself and
to leave objects unjudged, `counterpart-coverage` the first graded by
thresholds the rule states, and `effective-coverage` the first deciding by
a value without an upper bound
([#282](https://github.com/axioval/engine/issues/282)); `recess-width` is
the first of the plan-span capabilities
([#283](https://github.com/axioval/engine/issues/283)), judging the items
of a measured list against a row its provider selects from the rule's
table, and `light-well`, `centre-line-distance`, `component-visibility`
and `exit-separation` (whose required separation stays an interval over
both shares where its flag is unknown) follow, and `name-sequence` is the
first judging the items of a list on objects of their own (`at`), and
`numbering-consistency`, `wall-spacing`, `parking-bay`, `distance` and `containment` follow; `coordinate-consistency` is the first judging the sources themselves,
against a reference read once per rule, and `external-wall-validation`
the first leading one measurement to outcomes at source and at object
level ([#291](https://github.com/axioval/engine/issues/291)): every
capability of #282 now runs as a template; `space-boundary-coverage` is
the first wording everything one object leaves open in one outcome
([#284](https://github.com/axioval/engine/issues/284)).

## The outside contract

A template keeps, byte for byte:

- the capability id and parameter descriptor (names, kinds, requirement,
  grading, authored parameters), pinned for every built-in by
  `crates/engine/rules/tests/descriptors.rs`;
- how a rule's parameters are checked, and the rule-scoped outcome of a
  declaration it refuses or of a host missing its services;
- every finding and not-evaluated outcome: its scope, reason, count,
  wording word for word, evidence exactness and related objects.

The [parity harness](./parity.md) holds a template to all of it under
`Parity::contract()`.

## The template form

`axioval_engine::template::Template` is plain data, built in Rust by the
rules crate (`body_extent/template.rs`):

| Field | What it states |
| --- | --- |
| `id`, `parameters` | The capability's id and descriptor, unchanged. |
| `grades` | Whether findings are graded (the descriptor's `grades_deviation`): each states how far its value misses the bound it fails, measured from the declared bound, never the widened one, for the runtime's severity bands. |
| `name` | How rule-scoped messages name the capability (`body-extent: …`). |
| `refusals` | Where a refused declaration and missing services are reported: `rule` (the default, once, before anything is selected, after the name), `objects` (for each selected object, worded as the check states it, as capabilities that judged their declaration per object reported it) or `prefixed` (for each selected object, a refused declaration after a prefix: `slab-contact declaration is invalid: …`; missing services as stated) or `servicesPerObject` (a refused declaration once for the rule, after the name, missing services for each selected object) or `selected` (once for the rule after selecting, and only where the rule selects an object, beside the selection's own outcomes: a refused declaration after the name, missing services as stated, as capabilities that selected before reading their declaration reported it). |
| `defaults` | Values optional parameters take when unstated (`tolerance` 0 m). A default taking its value `from` other parameters takes the first of them the rule states, its literal where none is: one value a rule may state under either of two names (`horizontal` from `tolerance` or `horizontal_tolerance`), or the least severe of the thresholds stated. Such a name may be no parameter of the descriptor; messages, conditions and measured values (`@horizontal`) read it as any parameter. |
| `declaration` | `Check`s over the rule's parameters, in order: `excludes` (where a mode is stated, no string parameter of a list states an option it does not combine with), `arguments` (where a mode is stated, the rule parameters a measured value or member list names, checked as stated by the value's own argument check: a declaration, such as a table of rows, only the measurement knows how to read), `choice` (a string among options), `length` (a non-negative length), `count` (an integer of at least zero, stated where the descriptor requires it), `kind` (a parameter of its descriptor's kind, placed where the capability read it so refusals keep their order), `nonNegative` (numbers of at least zero), `traversal` (a valid `relationship` or `path`, declared only with one of the named parameters, or anywhere where it names none), `exclusive`, `anyOf`, `requires`, `ordered` (numbers, integers or quantities, as the descriptor types them), `disciplines` (a non-empty list of valid disciplines), `path` (a valid relationship path), `tolerance` (the rule's tolerance parameters valid together), `required` (a parameter stated, of its kind), `finite` (each parameter stated as a finite `number`, above or at least a bound where given), `increasing` (both numbers, the first below the second), `atMost` (each number stated at most a value: a share no greater than the whole), `when` (a check that applies only where one of some boolean parameters is stated true: a declaration a mode needs only while it is on), `amongEach` (every string a list states, trimmed, among options, `{value}` the one that is not), `declaresListed` (`declares`, a string list only where it lists one), `holds` (a `Condition` holds over the parameters, their defaults applied: a declaration stated as conditions, such as two switches not both off, or a share in `[0, 1)`), `exceeds` (where stated, a number above each earlier one stated: thresholds strictly increasing), `quantity` (where stated, a quantity of one dimension, of any sign), `angleBelow` (where stated, a plane angle of at least zero and below a number of degrees, compared in degrees as the capability converted it), `finiteLength` (a finite length of at least zero, `` `x` must be a finite length, not negative `` or `` `x` is not a length ``), `listed` (every string a list states is, as stated and untrimmed, one of some options and listed once, judged string by string in the list's order: `unknown` or `repeated`, `{value}` the string) and `listedNeeds` (for each string a list states, in its order, the parameters that option `Needed` are stated: a mode list whose options each need their own inputs). The first failing check leaves the rule not evaluated as an invalid declaration, worded as the capability worded it. `ifStated` judges a check only where the rule states a parameter (of any kind). A refusal worded after the template's name is not named again where its message names the capability itself (`horizontal-guard declaration is missing or not realisable`). |
| `services` | The host services the values need (`Service`: `object-frame`, `vertical-extent`, `triangle-count`, `walking-surface`, `contact`, `plan-area`, `plan-span`, `coordinate-system`, `envelope-membership`, `boundary-coverage`, `guard`, `space`, `free-space`), and the message leaving the whole rule open without them, before anything is selected (after selecting, with `selected` refusals). With `only` (a string parameter and a value), they are needed only where the rule states that value (`elevation_overlap` `overlapping`); with `whole`, a missing one leaves the rule as a whole open, after the name, whatever the `refusals`. |
| `texts` | Named message parts, optionally conditional: `Condition::Positive` (a parameter above zero), `Condition::Inexact` (a value read from evidence that is not exact, such as a count of a tessellation), `Condition::Equals` (a string parameter or its default is a value), `Condition::Zero` (a value's lower end is zero: nothing surely counted), `Condition::All` (every one of several), `Condition::Not`, `Condition::Cites` (a value's measured reads cite an object: a search found a candidate), `Condition::Absent` (a value is `null`: a measurement found nothing near), `Condition::Below` and `Condition::Above` (a value surely below or above a number: its upper or lower end is), `Condition::Lists` (a string-list parameter lists a word), `Condition::AtLeast` and `Condition::Under` (a parameter, or its default, a number at least or below a number: a tolerance not negative), `Condition::Exceeds` (a value's lower or upper end above the number a parameter states, never where it states none: a share surely or possibly above a threshold), `Condition::Stated` (the rule states a parameter, or a default gives it), `Condition::Measured` (a value was read: one read once per rule was not refused) and `Condition::Scope` (the source a scope judges is one a value's measured reads cite: the reference source), `Condition::Noted` (a value's measured reads noted something: boundaries that surely overlap). Several texts may share a name, each under its own condition: the first that holds is rendered (`plan area` or `facade area` by `measure`). |
| `forms` | The compositions. The first form whose `when` parameters are all stated applies. |

A `Form` holds:

- `values`: `TemplateValue`s, each a named expression, read in order for
  each selected object. A value may `expect` a kind (`length`, `area`), and words
  a stated absence (`absent`), a value of the wrong kind (`mismatch`) and
  one that cannot be read (`refused`, `{why}` its refusal);
  one expected `optional` that is stated absent passes the form, or the
  check, it belongs to without a finding (no slab above to measure to); one
  expected `words` is read only to word the outcome and cites nothing (the
  part uncovered beside its share, which cites the measurement). A
  measured value's reference to a parameter the template defaults binds
  the default, and `@selection` the objects the rule itself selects (the
  stack a slab belongs to); a rule reading that is never forked;
- `decision`: how the values decide;
- `fail` and `undecided`: the finding's message, and the message of an
  object the values cannot decide;
- `members`: where the form judges anchors through members (`Members`):
  the selector parameter picking them and what undecided members leave;
- `table`: the report table it fills (`Table`: a name and columns, each a
  value under an id that may name a text, in a quantity's dimension or a
  plain number, `template::NUMBER`), one row per selected object whose
  values were read, passing or not;
- `scope`: where the form decides when not for each selected object
  (`Scopes`): each source, or the whole project;
- `related`: the value whose measured reads' cited objects a finding
  relates (the doors a shelf length was measured with; see
  [rule parameters as arguments](./derived.md#rule-parameters-and-the-anchor-as-arguments)),
  or `members:<selector>`, the members one population surely picked (a
  ratio's numerator); without it, the decided members of every population,
  if any;
- `checks`: further decisions (`FormCheck`: values, decision, `fail`,
  `undecided`, `related`), judged in order after the form's values are read
  and before the form's own decision, each its own finding or not-evaluated
  outcome: a capability reporting one per failed check (D1). A check's
  value that cannot be read leaves only that check open; a form value that
  cannot be read leaves the object open once. A check `applies` only where
  its `Applies` holds (a measure the rule lists as consistent, a tolerance
  not negative), and decides by `Within` or by `Near`, its value against
  another value within a tolerance, as a member's judgement does. A check
  that does not apply is never read, nor read ahead, nor forked. A check's
  `derived` values follow its own and the form's values, before its
  decision: what only it reads (a capacity less an area). A check's
  condition reads the form's values and those read once per rule
  (`Condition::Measured`), so a check of a measurement the rule could not
  make is never read. Where its `unless` condition holds over its values,
  read, the check passes without deciding (a source declaring nothing
  external, whose own finding stands for its objects); a `quiet` check
  whose value cannot be read reports nothing, since another check reading
  it reports the refusal once;
- `unless`: values that leave an object unjudged (`Unless`), read before
  anything else, each only where it `applies` (a boolean parameter stated
  true): one surely true (`true`, or a number surely other than zero)
  passes the object without reading or measuring anything further, one
  surely false judges it on, one that may be either leaves it open with the
  form's `undecided` message. A storey the rule leaves out is such a value.
  Nothing is read ahead for a form with `unless` values, so an object left
  unjudged is never measured. A rule forked from it passes the object where
  a value that applies to the rule is other than zero (an `or`). A guard
  (`guard: true`) is only read, its value judging nothing: one that cannot
  be read leaves the object open, worded as refused (its `refused`
  message), before any other value is read, a list bound or a message
  worded, as a capability checked a precondition first; read ahead in
  batches like any `unless` value, it makes refusing an object that lacks
  the precondition cheap. A fork requires each guard defined (an `and`);
- `grading`: the severity of the form's finding (`Grading`, also on a
  check): `values` read once the decision fails (a `null` kept as one, a
  value that cannot be read leaving the object open as refused), then the
  severity of the first `Band` whose condition holds over the values, the
  rule's own where none does. The finding's message may read the values
  (`no contact` where the contact area is zero). Grading never changes
  which findings exist; a template grading a deviation (`grades`) is graded
  by the runtime's severity bands instead. A scope's finding takes the
  severity of the first band holding over its values;
- `once`: values read once per rule (`Once`), before any scope or object
  is judged (see below);
- `joined`: a separator that, stated, makes everything the form and its
  checks leave open on an object one not-evaluated outcome, their
  messages joined in order, for the first one's reason, after the
  object's findings: a capability wording all one object leaves
  undecided in one message;
- `project`: checks judged once for the rule after every selected object,
  whatever the rule selects (a project with no space still has storeys):
  their values and lists are measured of the project (a list's subject
  the project, `MeasuredProvider::members_of_project`), and an outcome is
  on the object its item names (`Items::at`, the item's first object of
  that field) or else the rule's own. They are graded after the form's
  checks, by their own `grading`.

A check that is `ungraded` states no deviation where the template grades
deviations: a capability reporting some findings at a fixed severity
beside others it graded.

**Rule parameters as arguments.** A value's measured name may hand the
measurement the rule's own parameters and the anchor, `@name` and
`@anchor` in a parameter's place (`shelf_length;…;doors=@door_selector`),
as an expression rule may ([Rule parameters and the anchor as
arguments](./derived.md#rule-parameters-and-the-anchor-as-arguments)).
Binding leaves a reference in place; the value binds it per object, the
selections read once per rule. A reference to an optional rule parameter
the rule leaves unstated, where the measured parameter is optional without
a default, is dropped when the rule binds (the value is measured as if the
argument were not written); any other unstated reference leaves the value
not evaluated.

**Scopes.** A form with `scope` decides once per source of the session
(an empty source included, `support::sources`), or once for the whole
project where the boolean parameter `Scopes::across` is true, over the
objects the rule selects there: the check an object rule cannot make,
since an object rule over an empty selection reports nothing. A value
reads the scope's objects as an aggregate over `Scopes::source()` (in the
catalogue, `selection`, the rule's own selection): the objects surely
selected are its members, those whose selection is undecided possible
members, so a `count` is an interval from the sure to the possible. The
`Within` judge decides it; a finding is scoped to the source or the
project, relates the objects surely selected and cites what selected
them, and a scope the possible objects leave undecided is not evaluated,
each of those objects too, for its own reason. With
`Scopes::disciplines` (a string-list parameter, checked by
`Check::Disciplines`), only sources playing a listed discipline count
(`SourceDisciplines`): a source declaring none is not evaluated per
source and its selected objects are possible members across sources.
`ScopeMessages` words the scope (`{place}`: ``in source `{source}` `` or
`in the project`), a project without a source to judge, a source whose
resource objects cannot be listed (`{why}`) and the discipline outcomes
(`{disciplines}`: `` `mep` or `hvac` ``). A form's `when` picks bounds
as for any form; an existence check is a form whose minimum is a value
`one`, a literal 1.

`Scopes::sources` says which sources are judged. `selected` (the
default) is every source over the rule's selection, as above. `every` is
every source of the session, an empty one included, without selecting: the
sources themselves are judged, by measured values whose subject is a
source ([Subjects](./derived.md#subjects)), read through a stand-in of the
source, so a source holding no object is measured all the same.
`occupied` is only the sources where the rule surely selects an object:
the rule's selection is made once and its outcomes reported, each such
source is judged (its values may name `@selection`, the rule's own
selection), and then every selected object is judged by the form's
`checks`, each its own outcome: one measurement leading to outcomes at
source and at object level. `reached` (`ScopeSources::Reached`, with a
selector parameter) is `occupied` and also every source where the rule's
selection leaves an object undecided or where that parameter's selector
surely picks an object: a check over those objects (whether any of a
source's walls declares itself external) concerns the source even where
the rule selects nothing there. A finding on a source relates the value
`related` names, if any. A scope reads the form's grading values
(`Grading::values`) only where it fails and its undecided ones
(`Grading::undecided`) only where it is undecided, before its message is
worded: what only a message words (how many walls a source holds) is not
read for a scope that passes. `Scopes::needs` judges the scopes only where a
condition holds over the values read once per rule (`Condition::Measured`):
where it does not, the rule's open outcomes stand for every scope.
`{source}` names the source a scope judges.

**Values read once per rule.** A form's `once` values (`Once`: a value,
where it `applies` over the rule's parameters, a `refused` message and
whether it is `required`) are read before any scope or object, of the
project: measured values whose subject is the project
([Subjects](./derived.md#subjects)), each read with what it cites (the
reference source of a federation, a building's envelope derived once).
Values applying where a list parameter lists a word (`Condition::Lists`)
are read in the order the rule lists the words. A refusal leaves the whole
rule open, once, worded by `refused` (`{why}` the refusal, for its reason),
or each selected object where the template reports refusals per object:
a `required` value's refusal leaves nothing else judged, any other's lets
the rule be judged on, and a check or scope reading it is gated by
`Condition::Measured`. A value read is available to every scope's and
object's messages and conditions under its name, its citations too
(`{reference:source}`, `Condition::Scope`). A `refusals: selected`
template reads them after selecting.

**Joined judgements.** `Decision::Joined` judges a subject by the items
of a measured member list (`Joined`: the list, written as a measured value
is, its `@` references bound; a truth field `found`, a text field
`words`, an optional truth field `recorded`, a `separator` and a `refused`
message), read for the subject: a scope's source (a list whose subject is
a source) or an object. Where any item is found, the subject is a finding
whose message (`fail`) reads `{found}`, the found items' words joined;
otherwise, where an item is undecided (or may be no item), the subject is
open (`undecided`) reading `{open}`, the undecided items' words joined,
each once: as not recorded where every one states `recorded` false, as
incomplete evidence otherwise. A list that cannot be measured leaves the
subject open with `refused` (`{why}`). The finding cites the list's
evidence. A source's coordinate differences, each in
`compare_coordinate_systems`' words, are such a list.

**Members.** A value reads an anchor's members as an aggregate over
`Members::source(selector)`, the objects the selector parameter picks: in
the catalogue it reads as written. Run, each anchor's members are its
reach (`counts::tally`, the one reader `related-count` and `keyed-limit`
share): the objects the selector picks that the rule's `relationship` or
`path` reaches from the anchor or, without one, every such object of the
anchor's own source but the anchor. The aggregate is narrowed to the
members surely picked, findings relate them, and their traversal evidence
is cited. Members the selector cannot decide leave the anchor as
`UndecidedMembers` says: `onlyExcess` (they can only add, as areas do)
keeps only a value surely above its maximum and leaves the anchor
otherwise not evaluated with its message (`{undecided}` the count,
`{relation}` how they are reached), and the anchor has no row: its value
is known only from below. `open` leaves the anchor open with its message
as soon as any member of any population is undecided, before anything is
read: a ratio is never judged over members that may be there. `widen`
makes them possible members of the aggregate instead: a count runs from the members surely picked to every
member that may be (as the evaluator widens any aggregate), and the
decision judges that interval, a verdict standing only where they cannot
change it. A member's value that cannot be read leaves the anchor open
worded as the member's measured value refused it.

**Two populations and ratios.** `Members::more` names further selector
parameters, each picking members of the same anchor along the same
traversal, read as an aggregate over `Members::source` of its own name
(a ratio's numerator and denominator); an unstated one picks none, and
their undecided members count with the first population's. A form's
`derived` values follow the values read: `Derived::Difference`
(`minuend − subtrahend`) and `Derived::Ratio` (`numerator / denominator`
of values of at least zero), both in plain binary arithmetic over the
intervals, as the capabilities computed them, and `Derived::Open` (a value
without an upper bound where another is surely above zero: a sum some of
whose parts could not be read, which a measured value, always finite,
states as the sum over what was read and how many were not). A ratio whose denominator
may be zero has no upper bound (infinity), where the evaluator's
division refuses a divisor holding zero, so a minimum it surely exceeds
still decides; a denominator surely zero leaves the object open with
`zero`. A form with derived values is not forked (its requirement, for the
catalogue, states a ratio as a division and a difference as a
subtraction); one with further populations forks each into an aggregate
along the same path, filtered by its selector. `area-ratio` runs on both,
and `relative-count` reads its two populations so;
`tests/template_features.rs` holds them to small templates of their own.

**Member checks.** `Members::checks` judge each member of the first
population on its own, the member in scope (`MemberCheck`: values, a
`Within` decision, `fail`, `undecided`, `open`), just before the anchor
reads the value the check names (`before`), and only where the anchor is
judged that far. A member whose first value is `null` or cannot be read is
not judged (the value the anchor reads refuses it where it must); a later
value that cannot be read leaves the member open (`open`, `{why}`). A
member found is a finding on the member relating the anchor, and one left
open is open, each reported once however many anchors reach it; once the
value `before` is read, an anchor with a member found is open as invalid
evidence (`failed`: `{failed}` how many, `{first}` the first). A light
area stated larger than its window is such a check of `area-ratio`.

`Members::every_when_unstated` reads an unstated selector parameter as
every object (`Selector::All`) rather than refusing the rule.
`Members::same_ends` names a string-list parameter holding a path: where
stated, only members from which that path reaches the same objects as
from the anchor are kept (`counts::same_ends`, a revolving door's swing
door between the same two spaces), a member whose ends cannot be read is
possible, an anchor whose ends cannot be read or reach nothing is open,
and `{relation}` ends `with the same ends via <path>`; `Check::Path`
refuses a path the step grammar does not read. The runner supplies an
anchor's members to the evaluator in place of the aggregate's source
(`ObjectLeaves::supplying`), in the project's order.

`Decision::Within` is the generic range judge: a value between a minimum
and a maximum, each a left-to-right sum of values and parameters
(`Term`), widened by `ROUNDING_ULPS` (four units in the last place) of the
largest `rounding` magnitude, an end of a value's or a parameter's
interval (`Magnitude`); without a magnitude (a count) the bounds are not
widened, and the expression form compares with them as they are. A
verdict needs the whole interval on one side of a bound, a straddling one
is undecided naming the bound. Each bound is decided by the one comparison
every rule uses (`axioval_engine::comparison::numbers`, through
`plan_area::judge`); only the bounds' arithmetic, a sum widened by the
allowance, stays plain binary, as the capabilities computed it, rather
than the evaluator's sound interval arithmetic, which rounds a widened
bound outward by an ulp and would leave open a value the capability
decided (see [Expand and fork](#expand-and-fork)).

`Decision::Compare` is the generic comparison judge: the stated value of
`value` against the target a rule names by an operator word
(`Comparison`): the operator parameter, the words testing presence, and
the target parameters in the order a statement is checked, each with its
kind (`TargetKind`) and the operator words it admits (`Operation`: an
order, `contains`, `matches`, `oneOf`, `noneOf`), and the parameters
folding case, stating a date precision and declaring a tolerance. Binding
refuses a statement in the order the capabilities refused it: exactly one
target (none for a presence word), an operator the target's kind admits,
`precision` for a date target only, a `matches` pattern that compiles, a
tolerance for a numeric target only. Per object it judges what the source
states, through the one comparison: an absence, `null` and a value of
another kind fail every operator but a presence test (which reads blank
text as undefined), a quantity against a unit-less target (or the
reverse) is not evaluated.

`Decision::Holds` is the truth judge: the value it names is a truth the
form's values compose with the evaluator's operators (`implies`, `and`,
comparisons, arithmetic over measured and stated values), and it holds.
`false` is a finding, `null` (Kleene's unknown, such as a comparison with
a value stated absent) leaves the object open with `undecided`. The values
read before the truth may be `null`: a stated absence is not a finding
there, since the truth decides what it means (`implies` over `isDefined`),
while a value stated `null` where a kind is expected (`expect`: `length`
or `area`) is of the wrong kind. The values after the truth are read only
where it fails, to word the finding, so a value only a failure needs is
never measured for a pass. A comparison in a truth is the evaluator's,
sound interval arithmetic: it decides as a capability's plain binary
comparison did except within a unit in the last place of a bound, which
it leaves open. Its expression form is the truth itself, so a rule forks
into it. Only the values up to the truth are measured ahead for many
objects at once; those after it are measured for an object that fails.

`Decision::Unique` is the generic group decision: each selected object's
stated value `value` compared with those of the other objects of its
group, a finding on every object sharing its value with another, relating
them. Groups are each source, or the project where the boolean parameter
`Unique::across` holds, narrowed by the rule's traversal to the objects
reaching the same related objects (`support::scope_key`: the spaces of
one storey). Text is compared trimmed (`Unique::trim`) and folded unless
`Unique::case_sensitive` holds (`support::value_key`); where the rule
declares a tolerance (`Check::Tolerance`), numbers and quantities of one
dimension are compared pair by pair within it, never transitively, or by
their rounding to `decimals`. A value stated absent, `null` or blank is a
finding worded `Unique::missing` where the boolean parameter
`Unique::require` holds, and is left out otherwise; one that cannot be
read leaves its object open as the property resolution refuses it. Its
finding reads `{others}` (how many objects share the value),
`{value:stated}` and `{tolerance:suffix}`. Its expression form, for the
catalogue, is that at most one object of the scope's `selection` states
the checked object's value; a rule is never forked from it.

`Decision::Consistent` is the second group decision: the objects stating
one value of the stated `key` must state one value of the stated `value`
(`Consistent`). Groups are each source, or the project where the boolean
parameter `Consistent::across` holds, of one kind unless
`Consistent::same_kind` is false, narrowed by the rule's traversal like
`Unique`'s; keys are trimmed and folded unless
`Consistent::case_sensitive` holds, and an absent, `null` or blank key is
a group of its own. An absent value is a value of its own. A group stating
two values is a finding on each object, relating those holding another
(`differs`, or `differs_unkeyed` for the group without a key;
`{others}` lists the other values). With the number parameter
`Consistent::tolerance` (numbers and quantities in SI units) or the
quantity parameter `Consistent::tolerance_quantity` (quantities of its
dimension), a numeric group agrees when its range, measured intervals at
their full width, is within the tolerance, widened by four units in the
last place of its largest magnitude; a range that may lie on either side
leaves each object open (`straddles`), and one beyond it is a finding on
each object farther than the tolerance from the group's median interval
(`beyond`, `{median}`), an object whose distance straddles it open
(`undecided`), or, where none is, on the objects at the range's ends
(`at_end`, `{range}`). A value the tolerance does not apply to leaves its
object open (`inapplicable`, `not_finite`, `inexact`). Messages read
`{objects}` (the group as `keyed` or `unkeyed` words it) and `{spread}`
(the tolerance as declared). Its expression form, for the catalogue, is
that no object of the scope's `selection` states the checked object's key
with another value; a rule is never forked from it.

`Decision::Conforms` is the third: each selected object satisfies the
selector parameter `Conformance::requirement` (an agreed list, an `anyOf`
of `allOf` rows), decided by the selector evaluation every rule shares;
an object it cannot decide is left open for the selector's reason. The
objects it rejects are worded by the stated values of the properties the
selector consults, in the order it names them (a related selector or a
pattern names none): an object none of whose consulted properties has a
value is a finding of its own (`no_value`, `{properties}`), and the
objects holding one combination of values (text folded, an empty value
as no value) share one finding on the first of them, relating the others
(`unknown`, `{values}`). A selector consulting no property, or one whose
properties cannot be read, finds the object alone (`alone`). Its
expression form names the test (`` `requirement` holds ``), since no
expression applies a selector to the object in scope; a rule is never
forked from it.

`Decision::Proportion` judges two exact counts by a proportion the rule
states (`Proportion`, its parameters named by `ProportionParameters` and
checked by `Check::Proportion` in the capability's order and words), in
integer arithmetic: a ratio, `provided / provided_unit` standing in an
operator word to `required / required_unit` (compared cross-multiplied),
with an exception for a required count from 1 below
`small_required_below`, judged as `provided operator small_provided`; or
a table of rows `R:P` (from `R` required, at least `P` provided), each
further `additional_required` beyond the last row needing
`additional_provided` more, nothing required below the first. The counts
are an anchor's two member populations (`Members::more`), whose undecided
members leave the anchor open before anything is judged
(`UndecidedMembers::Open`, `{undecided}` and `{relation}`); or, with
`Proportion::groups`, the groups the rule's selection forms by a stated
value per source or across sources (`ProportionGroups`): an object in
both the selection and a population is counted, one either may pick only
possibly, and a group with a possible member, or in a scope holding an
object whose value cannot be read, is open, one with required objects and
none provided is found as such. Messages read `{requirement}` (`1/1
at_least 4/1`, `at least 2 provided for 5 required`), a group's
`{label}`, `{provided}` and `{required}`. Its expression form, for the
catalogue, is the ratio mode, a branch per operator word; a rule is never
forked from it, since small counts, tables and groups have no expression
form the evaluator decides alike.

`Decision::Facets` is the facet judge: a property's value against lexical
constraints written as XML Schema facets, cast to the kind of the value
the source resolves (`FacetParameters` names every parameter). The
property is the reference parameter `property`, resolved exactly with its
declared type (which the evaluator's values do not carry), or every
property whose name matches `property_pattern` and set
`property_set_pattern`, enumerated exactly. Binding refuses a declaration
as the capability did, the property's refusal after `target_refusal` and
a constraint's after `constraint_refusal`, which is why such a template
words its refusals itself (`Refusals::Worded`). Each value is judged
against `data_type`, `values`, `patterns`, the four bounds (through the
one comparison), the lengths and digits, under `optional`, `precision`,
`quantifier` and `si_units`; the judge words its findings and open
outcomes as the capability did. Its expression form names the test;
a rule is never forked from it.

`Decision::Requirements` is the requirements-table judge
(`RequirementParameters`): every applicable row of a table per selected
object, each naming a property exactly, by wildcard or by XML Schema
patterns over set and name, and stating a requirement (`required`,
`optional`, `forbidden`) or a state row's presence or value conditions
(wildcards, lists, substrings, numeric ranges in a unit, per a measured
or stated area or volume and rounded, date ranges), restricted by
`applies_to`. Binding parses every row, refusing one as the capability
did (`row <n>: …`, after the template's name). Each failing row is its own
finding (divergence D1's several findings per object), worded by its
result (`missing property`, `forbidden value`, `wrong value`, …), prefixed
by the object's category where `category_property` names one, and one
row's findings of one value merged with `group_by_value`. Divisors are
measured through the plan-area readers every area capability shares. Its
expression form names the test; a rule is never forked from it.

`Decision::Compared` is the candidate comparison judge
(`ComparedParameters`): a property of the candidates each checked object
reaches (itself, the members of a group it shares, the objects the rule's
traversal reaches, or those sharing its nearest container, across sources
by level) compared with one target of the checked object, a property, a
constant, a text list or a range, scaled by a factor, under the rule's
tolerance and precision, through the one comparison; each candidate, at
least one, their count or their sum. Binding refuses a declaration in the
capability's words after `refusal` (`Refusals::Worded`). A judge checking
its own declaration (`Facets`, `Requirements`, `Compared`) is bound before
any parameter is read as a constant, so its refusal comes first, as the
capability's did. Its expression form names the test; a rule is never
forked from it.

`Decision::Each` judges an anchor's members one by one (`Each`), with
three relations no aggregate states:

- **Neighbouring members.** The members (`Form::members`, decided as
  `UndecidedMembers::Refuse` requires: any object the selector cannot
  decide, anywhere, leaves every anchor open with `{why}`) read `values`
  with themselves in scope and are ordered by `order`, lowest first and
  ties by identity; one whose `order` is not a stated length leaves the
  anchor open (`unordered`). A member's `rise` is the next member's
  `order` less its own; the last member's comes from `Rise::last`, the
  highest value of a nested population (a level's contents' tops), or
  leaves it open (`Rise::open`); `skip_first`/`skip_last` leave the first
  or last without one. The rise is read only where an applicable
  judgement reads it.
- **A prevailing reference.** `Decision::Near` judges a value against a
  reference within a tolerance, the reference a value of the subject, of
  its member (`member:<name>`), or `Reference::Prevailing`: the exact
  value most subjects share within the tolerance, the lowest among
  equally common ones (`level_spacing::prevailing`); without one each
  subject is open with `missing`, or nothing is judged.
- **Nested members.** `Nested` reads, per member, the objects a selector
  parameter picks (every object where unstated) that a path parameter
  reaches (a storey's spaces): any undecided one leaves the member open
  (`undecided`), fewer than `least` judge nothing or leave it open
  (`fewer`), the `services` they need are checked before or after
  reaching them, and a value that cannot be read leaves the nested member
  open, or the member (`errors_open_member`). `differences` derive values
  (`top − bottom`), and a `NestedTable` holds a row per nested member, the
  member's identity first.

Every `Judgement` (`Each::checks`, `Nested::checks`) is its own outcome on
its subject, so a member failing two judgements has two findings
(divergence D1's "several findings per object"); one applies where its
`Applies` holds (every `when` parameter stated, a boolean or its default
true; one of `any`; its `condition`, such as `Condition::OneOf`), and only
to subjects having every value it reads, at least `least` of them. A
member's findings relate the next member up, a nested member's its
member. Rises and differences are computed in plain binary arithmetic
(`[a.lower − b.upper, a.upper − b.lower]`), as the capabilities computed
them, like `Within`'s bounds. Such a form expands into the conjunction of
its judgements for the catalogue and is never forked.

The declaration checks such a capability needs are `nonNegativeLength`
(a non-negative length, worded by the template), `together` (all or
none), `among` (a string among options, `{value}` the stated one),
`declares` (one declared: stated, a boolean true, a table with a row) and
`falseRequires` (a boolean stated false needs one of some parameters).
The opening templates add `nonNegativeQuantity` (a quantity of a
dimension, at least zero: an area tolerance).

**Items of a measured list.** A form's check may decide
`Decision::Items` (`Items`): the items of a measured member list, the
list written as a measured value is (`landings;of=ramp;landing=@landing_objects`,
its `@` references bound for each object, an unstated optional one
dropped), read once per object however many checks read it. A list that
cannot be measured leaves the check open with `refused` (`{why}`), for
the reason the list was refused (an approximate footprint is invalid
evidence, an unavailable one incomplete), or reports nothing where another
check already did. Each item is judged by
`ItemCheck`s, every failing test its own outcome (D1):

- a `Test` judges a `Range` (a number field between the largest of its
  `at_least` and the smallest of its `at_most` requirements, each the
  first `Choice` whose conditions hold and whose bound is known: a rule
  parameter, a number of the item, or a literal; widened by an
  `Allowance`: eight units in the last place of a magnitude, at least one,
  `times` over, the largest magnitude of any item judged together, or an
  allowance the measurement states, or `raised`: a fixed allowance added
  to the value rather than to the bound, `value + allowance ≥ bound`, as a
  capability raising a measured length by its rounding compared it), a
  `Truth` (one value a finding), the
  `Rows` of a table parameter (an item fails where every row fails,
  graded by the row it misses least), or `Fails` (a finding wherever its
  conditions hold: a requirement stated as conditions). A finding grades
  its deviation from the declared bound; a bound that is an interval (an
  item's own width) fails at its most lenient end and passes at its
  strictest. `then`, `otherwise` and `straddled` judge a further test in
  a test's place where it passes, fails or is open; `Effect`s turn a
  pass, a finding, a finding below a lower bound, or a `null` open where
  their conditions hold (`{failed}` the finding's words); a range's `null`
  that is `unmet` fails the test whatever bounds it, `otherwise` in its
  place (no barrier covers the edge, so it is not covered);
- a `Group` reads `Guard` fields first: an undecided one leaves the item
  open once for all its tests, a `null` one is skipped, opened or failed;
- `When` conditions read the rule (`declared`, `undeclared`, `equals`,
  `undecided`: a selector parameter leaving objects undecided) or the
  item (a truth field, a `null`, a number exactly or surely below a value,
  an empty objects field, an undecided field: `unknown`).

An `Items` check naming `at` (an objects field) reports each item's
outcomes on the object it names, in place of the checked object: an
opening a corridor reaches. A list that cannot be measured leaves the
check open for the list's own reason.

An `Items` check with `once` leaves the object open at most once: of its
open outcomes only the first is reported, and none where an earlier check
of the form already left the object open, as a capability reporting one
doubt per object did.

With `combined` (`Combined`), every item's outcome is one outcome on the
object: where any item fails, one finding whose message reads
`{findings}`, the failing items' messages joined by its separator, graded
by the worst and relating every object they relate; otherwise, where any
is open, one open outcome (`{opens}`) for the first one's reason;
otherwise a pass. Its words read the first item's fields, which every
item shares (a window's limit row, judged against each floor beside it).
With `alternatives` (`Alternatives`), the items of one group (a text
field) are the candidates for one unknown (the floor a door's side steps
onto), those whose truth `sure` holds surely among them: the group fails
where a sure item fails, or where none is sure and every item surely
fails; it passes where every item passes; otherwise it is open with its
items' open outcomes, or with its own words where none is open. A measured
member may carry evidence of its own (`MeasuredMember::evidence`, the
floor it was measured above): an outcome on that member cites it, an
outcome on another never does.

An `Items` check naming `reason` (a text field) leaves an item that states
one open for that reason, as a report writes it (`missing_service`): a
placement whose body cannot be read, a search missing its service. Every
other open outcome is for incomplete evidence.

With `merged`, the findings of several items worded alike are one
finding on the object, relating every object each related (sorted, each
once): one finding per distinct defect, not one per item. A form's check
deciding by `Items` is graded by its `grading` like any check, every
finding of its items at the band's severity. A template's own
`@selection` is always bound, never dropped as an unstated parameter.

`Together` judges every item in one outcome instead, only the items whose
field `present` is stated and where its `when` conditions hold: `every`
number in a range, named `riser {index} of {count}` among them, the failing
ones in one finding graded by the worst and the open ones grouped or each
on its own (a number surely within `zero` of zero is no item: a straight
tread is no winder); every truth (`truths`); the `spread` of the numbers
(the largest less the smallest) against a tolerance; their `count` within
bounds (`{count}`); their `least` against a minimum, the least between
the least lower and the least upper end, at the item of least upper end
(`{least}`, `{at}`), an unmeasured item (`{unknown}`) only able to lower
it; or `any`, one item meeting a requirement (`Any`, its conditions
`holds`): where none holds, the first item matching one of the `open`
cases leaves the check open (`{why}` an undecided field's reason), and
where none is open the items matching `fails` are one finding
(`{failing}`), relating each one's objects.
`Passing` opens the check where every item passed and a condition holds
(`undecided`): once, or per item of another list grouping them by a key
(each stretch whose rails all pass). Messages read item fields
(`{label}`, `{governing}`, `{governing:and}`, `{rise:length}`,
`{slope:ratio}`, `{angle:degrees}`, `{area:area}`: square metres rounded
to 1e-4, `ItemUnit::Area`), parameters (`{minimum:length}`),
the chosen requirements (`{stated:length}`, `{requirements}` their words
joined by ` and `), `{bound}` (`0.15 m to 0.19 m`), `{bound:plain}` (the
bound a range failed or straddled, its number as declared and without a
unit, as the range judge words it: `at least 4`), `{why}` and the
`ItemText`s, each under conditions. A form with such a check is never
forked (`ForkError::Inexpressible`); its expression form is a `none`
aggregate over the list of an item failing its tests.

An item a list leaves undecided for a reason of its own (a missing
service, invalid evidence) states it in its `reason` field, read through
`Items::reason` (`measured_kinds::refused_field`). With `Items::joined`, a
separator, an item's
failing tests are one finding, their messages joined, relating and citing
what each does and graded by the worst deviation where each states one,
or, where none fails, the first left open: a capability reporting one
outcome per object for all it checks of it (an item placed with
`Items::at` is placed as one).

The declaration checks the stair and ramp templates add are `positive` (a
length above zero), `needs` (where a check is declared every parameter it
needs is stated, and none of them without one), `below` (two lengths in
order), `requiresValue` (a parameter only with another's string value),
`requiresDeclared` (only with a declared one), `rows` (each row of a
table states its columns as required), `angle` (a plane angle of at least
zero), `valueRequires` (a string value needing other parameters),
`togetherExcept` (stated together but for an exception) and `anyRequires`
(any of some declared needs another). Booleans, integers and tables are
read as the capabilities read them, a value of another type refused.

**Parts.** A form deciding `Decision::Parts` (`Parts`) reaches each
selected object's parts, the objects a selector parameter picks along a
path parameter, and judges each part once, by the first object reaching
it: its `values` first (one that cannot be read leaves the part open
once), then its `checks`, every outcome on the part; its measured values
name that object as `@anchor`. The form's own values and checks then judge
the object. A path the object cannot be read along leaves it open. Such a
form is never forked.

### `object-count`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| bounded | `minimum` | `count` = the count of the scope's `selection` | `count` within `minimum` and `maximum`, no rounding |
| at most | `maximum` | `count` | `count` within `maximum` |
| existence | always | `count`, `one` = 1 | `count` at least `one` |

Each form decides per source, or for the project with `across_sources`,
over the sources playing `disciplines` where stated (`Scopes`). The
declaration checks both bounds' kinds, that neither is negative, their
order, `disciplines` and `across_sources`, in the capability's order. The
finding reads `{matched}; required {required:exactly}`, the text
`matched` being `no object matches the selection {place}` where nothing
surely counts (`Condition::Zero`) and `{count:least} object(s) match the
selection {place}` otherwise; a straddled bound leaves the scope open
with `object-count: {count:least} object(s) match the selection {place}
and {undecided} more may; required {required:exactly}`, each undecided
object open for its own reason.

### `related-count`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `count` = the count of the anchor's members (`related_selector`, every object where unstated) | `count` within `minimum` and `maximum`, no rounding, graded |

Members are reached along the rule's traversal, or everywhere in the
anchor's source, kept to those sharing their ends with `same_ends`, and
widened by the undecided ones (`UndecidedMembers::Widen`). The
declaration checks both bounds' kinds, that one is stated, that neither
is negative, their order, the selector's kind, the traversal and the
`same_ends` path. The finding reads `{count:least} related object(s)
{relation}; required {required}` (`via bounds`, `in the same source`, `…
with the same ends via bounds:backward`); a straddled bound leaves the
anchor open with `{count:least} related object(s) {relation} and
{undecided} more that may count; required {required}`.

### `unique-value`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `value` = the stated `{property}` | `Unique`: groups per source or across sources (`across_sources`), narrowed by the traversal; `trim` (default true), `case_sensitive` (default false), `require_value` (default true) and the tolerance parameters as declared |

The declaration checks `property`, the four flags' kinds, the traversal
and the tolerance, in the capability's order. Its findings read
`{property} {value:stated} is also used by {others} other object(s)
{tolerance:suffix}` and, for an object without a value, `{property} has
no value`.

### `consistent-value`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `key` = the stated `{key}`, `value` = the stated `{value}` | `Consistent`: groups per source or across sources (`across_sources`), of one kind unless `same_kind` (default true), narrowed by the traversal; `case_sensitive` (default false); `tolerance` or `tolerance_quantity` on numeric values |

The declaration checks both tolerances' kinds, that only one is stated
and neither is negative, `key`, `value`, the three flags' kinds and the
traversal, in the capability's order. Its findings read `{value}
{value:stated} where other objects with {key} {key:stated} have
{others}`, `{key} has no value, and {value} is {value:stated} where other
objects without {key} have {others}`, and under a tolerance `{value} is
{value:stated}, farther than the tolerance {spread} from the median
{median} of the {objects}` or `…, at an end of the range {range} of
{value} over the {objects}, which exceeds the tolerance {spread}`.

### `selector-conformance`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | none | `Conforms`: each object by `requirement` |

The declaration checks `requirement` and the kind of `message`, which
defaults to `does not match any agreed combination of values`. Its
findings read `{message}: {values}` for a combination of values,
`{properties} has no value to compare with the agreed list` for an object
stating none, and `{message}` alone where the selector consults no
property.

### `relative-count`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| groups | `group_property` | `group` = the stated `{group_property}` of each object | `Proportion` per group of the rule's selection, per source or across sources (`across_sources`), `case_sensitive` |
| anchors | always | `provided`, `required` = the counts of the anchor's members `provided_selector` and `required_selector` pick (`Members::more`), open where any is undecided | `Proportion` of the two counts |

The declaration checks both selectors, the proportion (`Check::Proportion`:
ratio or table, in the capability's order and words), then the grouping:
`group_property`'s kind, `across_sources` and `case_sensitive` only with
it, no traversal parameter with it (each refused by name), the traversal
and both flags' kinds. An anchor's finding reads `{provided:least}
provided and {required:least} required object(s) {relation}; required
{requirement}`, and one with undecided members is open with `{undecided}
related object(s) {relation} cannot be assigned to either population`. A
group's outcomes start `group {group_property} {group:stated}`.

### `property-value`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | none: the facet judge reads the property resolution | `Facets` over `property` or `property_set_pattern`/`property_pattern`, with every facet parameter |

Refusals are worded in full (`Refusals::Worded`): the property's after
`property-value:` (``declare `property` or `property_pattern`, not
both``), a constraint's after `property-value parameters are invalid:`
(`no data type and no value constraint`). Findings and open outcomes are
the facet judge's, as the capability worded them.

### `property-requirements`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | none: the judge reads the property resolution and enumeration | `Requirements` over `requirements`, `case_sensitive`, `area_property`, `volume_property`, `group_by_value`, `category_property` |

A refused row reads `property-requirements: row <n>: …`; findings start
with their result and end `(requirement row <n>)`, as the capability
worded them.

### `property-comparison`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | none: the judge reads the candidates' and the target's properties | `Compared` over every parameter |

A refusal reads `property-comparison parameters are invalid: …`
(`Refusals::Worded`); findings are the judge's, as the capability worded
them (`candidate … does not satisfy comparison`, `no candidate satisfies
comparison`, `sum of compared values is … and is not …`).

### `level-spacing`

One form, `Decision::Each` over the anchor's levels (`member_selector`,
refused while it cannot decide an object):

| Part | What it reads | Judgements |
| --- | --- | --- |
| members | `elevation` = the stated `{order}`, a length; `height` = the rise to the next level up, the highest's from `contents` (`ignore_lowest`, `ignore_highest` skip an end) | `height` within `minimum` and `maximum`; with `consistent`, `height` near the prevailing height within `tolerance` (1 mm by default), over at least two heights |
| `contents` | with `content_path`: the `content_selector` objects it reaches, each its measured `top` | the highest top less the elevation is the highest level's height |
| `space elevations` | with `space_elevation`: the `space_selector` objects `space_path` reaches, at least two, each its measured `bottom` and `top` | each `bottom` (`bottom`, `both`) and `top` (`top`, `both`) near the prevailing one among the level's spaces within `space_tolerance` |
| `space heights` | with `space_height` (true by default): the same spaces of each level with a height | each space's `top − bottom` near its level's height within `space_tolerance`; the table `spaces` |

The table `levels` holds every level's elevation and height (unknown for
one not measured). The declaration keeps the capability's checks and
wording, in its order. Messages are the capability's: `level height is
{height:length}; required {bound}`, `level height {height:length} differs
from the prevailing {reference:length}`, `space bottom elevation is …`,
`space height is {height:length} and its level's height
{member:height:length}; …`.

### `property-predicate`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `actual` = the stated property `{property_set}`.`{property}` | `actual` compared by `Compare`: `operator` against one of `value` (integer), `number`, `quantity`, `date`, `date_time`, `boolean` (equality), `texts` (`one_of`, `none_of`) or `text` (`equal`, `not_equal`, `contains`, `matches`); `is_defined`/`is_undefined` take none; `case_sensitive`, `precision` and the tolerance parameters as declared |

Its finding reads `property {property_set}.{property} does not satisfy
{operator}{target}{tolerance:suffix}; actual value is {actual:stated}`. A
target computed per object (`per_object`) is evaluated to a literal per
object first (`object_parameters::per_object`), as before.

### `body-extent`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| stated length | `target_property` | `extent` = `body_extent;axis={axis}`, `low`/`high` = `body_position;axis={axis};end=low\|high`, `target` = the stated property, a length | `extent` within `target − tolerance` and `target + tolerance`, rounding over `low`, `high`, `target` and `tolerance` |
| range | always | `extent`, `low`, `high` | `extent` within `minimum` and `maximum`, rounding over `low`, `high` and `extent` |

`body_position` is a measured value of its own: where the body begins and
ends along its axis, the magnitudes the binary rounding of the extent
scales with. The capability allowed that rounding all along; the template
states it, so a wall exactly at a bound far from the origin is judged as
before.

### `triangle-count`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| maximum | always | `count` = `triangle_count` | `count` within no minimum and `maximum`, no rounding |

The declaration is one `count` check (`maximum` stated, at least zero);
the service is `triangle-count`. The finding's message appends the text
`tessellated` (the count depends on the host's tessellation) where the
count's evidence is not exact (`Condition::Inexact`), as the capability
said so on a tessellation of curved faces. The finding cites the measured
`triangle_count`, whose evidence carries the host's count locator and its
exactness.

### `plan-area`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| members | `member_selector` | `area` = the sum over the members of `plan_area;measure={measure}`, in m² | `area` within `minimum` and `maximum`, no rounding |
| own | always | `area` = `plan_area;measure={measure}`, in m² | the same |

`measure` defaults to `footprint`; the texts `noun` (`plan area` or
`facade area`) and `column` (`plan_area` or `facade_area`) follow it. The
declaration checks the bounds' kinds, that one is stated
(`minimum or maximum is required`), that none is negative, their order,
the member selector's kind, the traversal (only with `member_selector`)
and `measure`, in the capability's order. The template grades, fills the
table `areas` with the area in the column `{column}`, and judges anchors
through their members with `onlyExcess`. An area is read as a plain number
of square metres (the measured area divided by 1 m²), as the bounds are
stated, so the forked rule compares like with like. The sum of members is
the evaluator's exact interval sum: where the binary sum rounds, its
interval holds the capability's rounded sum and is a unit in the last
place wider (D19).

### `shelf-capacity`

| Form | When | Values | Checks | Decision |
| --- | --- | --- | --- | --- |
| shelving | always | `length` = `shelf_length;…` in metres | `height` = `shelf_clear_height;…` in metres, at least `top_elevation_metres`, no rounding | `length` at least `minimum_running_metres`, no rounding |

Both values name the rule's arrangement and selectors as arguments:
`depth=@shelf_depth_metres;horizontal=@horizontal_spacing_metres;vertical=@vertical_spacing_metres;bottom=@bottom_elevation_metres;top=@top_elevation_metres;clearance=@door_clearance_metres;access=@access_path;doors=@door_selector;openings=@opening_selector;spaces=@space_selector`,
an unstated `opening_selector` or `space_selector` dropped. The provider
measures both from one request per space with the doors and openings the
access path reaches it from, the selections bound into it with their
undecided objects (an undecided door that may reach the space leaves it
open), and cites the doors it sent: the finding on the length relates
them (`related`). The declaration is refused per space (`refusals:
objects`), in the capability's order and words: the minimum (`finite`, at
least zero), the arrangement (`finite` and `increasing`, one message), the
selectors needing `access_path` (`requires`), `access_path` and a door or
opening selector (`anyOf`); a malformed access path is refused by the
measured value, as an invalid declaration worded as the capability worded
it. A space too low for the shelving and one too short are two findings
(the check, then the form's decision), as the capability reported them.
Messages show the length's upper end and the minimum to three decimals
(`{length:upper3}`, `{minimum_running_metres:fixed3}`). The provider keeps
the access index and each space's shelving for the run, so both values
read one request per space ([Template performance](./performance.md)).

### `area-ratio`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| light areas over members | `numerator_derivation`, `denominator_selector` | `reached`, `null` where `empty_numerator_finding` holds and the anchor reaches no numerator member; `numerator` = the sum over the numerator's members of `light_area;…`; `stated_count`, `table_count`, `frame_count` = the sums of `light_step;…;step=stated\|table\|frame`; `denominator` = the sum over the denominator's members of `ratio_area;property=@denominator_property;measure=@denominator_measure;otherwise=@measure` | `ratio` = `numerator / denominator` (`Derived::Ratio`) within `minimum` and `maximum`, no rounding, graded |
| light areas over the anchor | `numerator_derivation` | the same, `denominator` the anchor's own `ratio_area;…` | the same |
| members | `denominator_selector` | `reached`; `numerator` = the sum of `ratio_area;property=@numerator_property;measure=@numerator_measure;otherwise=@measure`; `denominator` as above | the same |
| own | always | `reached`, `numerator`, the anchor's own `denominator` | the same |

`light_area`, `light_size` and `light_step` name the light-area chain as
the rule declares it (`stated=@numerator_property;overall_width=@overall_width;…;frame_width=@frame_width`),
each key the rule's parameter of that name; the rule's table and property
references bind as stated. The members are the `numerator_selector`
objects the rule's traversal reaches, and the `denominator_selector`
objects (`Members::more`); any undecided one leaves the anchor open with
`{undecided} related object(s) {relation} cannot be assigned` before
anything is read (`UndecidedMembers::Open`). A finding relates the
numerator's members (`members:numerator_selector`) and reads
`{noun} ratio is {ratio:area} ({numerator:least2} m² of {denominator:least2} m²); required {bound:plain}`,
in the light-area forms followed by `{provenance}` (`; light areas: 1
stated, 1 from the light-area table`, composed of texts over the step
counts with `Condition::All` and `Condition::Not`); `noun` is `plan area`,
`facade area`, or `facade area to plan area` and the reverse, by the
measures. A denominator surely zero leaves the anchor open with `the
denominator has no {bottom}`. In the light-area forms a member check
(`before` the numerator) compares a stated light area
(`light_area;…;read=stated`, none where another step gave it) with the
member's overall area (`read=overall`), within four units in the last
place: `light area {light:area} m² ({numerator_property}) is larger than
the overall area {overall:area} m² ({overall_width} {width:area} m ×
{overall_height} {height:area} m)`, the anchor open with `{failed}
member(s), first {first}, state a light area larger than the element`; an
overall size that cannot be read leaves the member open (`the light area
cannot be compared with the overall area: {why}`). The declaration keeps
the capability's order and words: the light-area parameters are refused
without the mode (`requires`), the mode's own checks are the light-area
chain's (`arguments`, `light_area::check_arguments`), and a facade measure
does not combine with it (`excludes`). The table `ratios` holds
`numerator_area`, `denominator_area` and `ratio` (a plain number).

### `plan-coverage`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `share` = `plan_coverage;candidates=@candidate_selector;minimum=@minimum_ratio;relationship=@relationship;direction=@direction;path=@path;follow_chain=@follow_chain;skip_absent_relationship_ends=@skip_absent_relationship_ends` | `share` at least `minimum_ratio`, no rounding |

`plan_coverage` is the capability's search as a measured value
(`plan_coverage::search`): the candidates the rule's selector picks that
its traversal reaches (each traversal parameter named as the rule names
it, unstated ones dropped), searched in the order reached for one surely
covering `minimum` of the footprint. Its value is the share as far as the
search needs it: the covering candidate's lower share, where one is found;
otherwise from the largest lower share to the best candidate's upper share
or, where a share or an undecided candidate may reach the minimum, the
minimum at least; it cites the best candidate, which a finding relates
(`related: share`). The declaration checks the minimum as a share
(`finite` above 0, `atMost` 1, one message), the candidate selector and
the traversal. The finding reads `at most {share:share} of the footprint
lies within any {candidate}; required {minimum_ratio}`, `candidate` being
`candidate (there are none)` where the search cited none
(`Condition::Cites`); an undecided one `coverage of {minimum_ratio} cannot
be decided from the measured areas`.

### `ramp-geometry`

One form: the ramp's `run_count` read first, so a ramp the walking-surface
service cannot measure is open once, for its reason; then a check per
declared requirement, each `Items` over a list the ramp's runs are
measured into:

| Check | List | Judgement |
| --- | --- | --- |
| `slope_limits` | `runs` | each run against the table's `Rows` (slope, length, rise), its slope's rounding stated by the run |
| `slope_tolerance` | `runs` | the `spread` of the run slopes, twice the largest slope rounding |
| `minimum_headroom` | `clearances;side=above` | the clearance at least the minimum; an undecided obstacle opens a pass or nothing above |
| `width_minimum`, `width_maximum` | `runs` | `every` run width, open where one is not measured |
| landings | `landings` | the landing present where required; its depth and width at least the minimum (the end minimum at the two outermost ends) and the run's width, open once without a rectangle or a measured width; an undecided carrier opens a shortfall |
| `landing_doors`, `landing_door_swing` | `landing_doors`, `landing_swings` | the searches' answers |
| `minimum_headroom_below` | `clearances;side=below` | as headroom, over the spaces' floors |
| handrails | `handrail_stretches`, `rail_heights`, `rail_extensions`, `rail_gaps` | the sides required (one, both, both above a width); each rail's height; each extension and its rail running level over the minimum; each gap; an undecided rail opens a pass per stretch, a missing side, a short extension or a gap |
| `check_continuous_handrails`, `check_rails_obstruction`, `end_space_*` | `rail_continuity`, `rail_obstructions`, `end_spaces` | the searches' answers |
| `clear_width_minimum` | `clear_widths` | each run's clear width at least the minimum; an undecided obstacle opens a pass |

The declaration keeps the capability's checks, words and order. Searches
(the free space at each end, doors on or over a landing, the handrail
across a landing, rails over a surface) answer three-valued per item and
word what they found; the template judges the numbers. Their counts
(`obstructed_end_spaces`, `landing_door_conflicts`, `handrail_breaks`,
`rails_over_surfaces`) sum the same items rather than running the
capability.

### `stair-geometry`

Two forms. With `stair_path`, `Decision::Parts`: each stair's flights
(`stair_flights` along `stair_path`) judged once as parts, by every flight
check below, their tactile strips and landing clear widths knowing which
ends lie between two of the stair's flights (`stair=@anchor`); then the
stair by its rise (`stairs`: the rise from its lowest flight's base to its
highest flight's top, a pass open where a flight may be missing), the
handrail across its landings (`stair_continuity`, a search) and its least
clear width (`stair_clear_widths`, `least`). Otherwise each flight, its
`flight_rise` read first (one that cannot be measured is open once):

| Check | List | Judgement |
| --- | --- | --- |
| `riser_*`, `going_*`, `nosing_*`, `step_length_*` | `steps` | `every` riser, going, nosing, step (`2r + g`, three times the allowance) in its range |
| `minimum_risers`, `maximum_risers` | `steps` | their `count` |
| `maximum_rise`, `width_*` | `flights` | the flight's rise and width (open where a tread fills no rectangle) |
| `riser_tolerance`, `going_tolerance` | `steps` | the `spread` of the risers and goings |
| `winder_angle_*` | `steps` | `every` winder angle at most the maximum; on a turning flight, every winder (no straight tread) at least the minimum |
| `forbid_open_risers` | `steps` | the `truths` of the open risers |
| headroom, landings, doors, end spaces, clearance below, handrails | as `ramp-geometry`'s, of the flight | as `ramp-geometry`'s |
| `tactile_*` | `tactile_strips` | the searches' answers, an intermediate end only with `tactile_on_intermediate_landings` |
| clear widths | `clear_widths` | the flight's and each landing's at least their minimum; the `least` of all at least `total_clear_width_minimum` |

The declaration keeps the capability's checks, words and order, the
whole-stair mode's included. Per-object parameters are evaluated per
object first, as before. The defect counts sum the same items:
`missing_tactile_strips` the strips (an intermediate end only with
`intermediate=yes`), `handrail_breaks` of a whole stair its breaks.
### `slab-contact`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `share` = `contact_share;with=@counterparts;side=@contact_side;gap=@maximum_gap_metres;intersection=@maximum_intersection_metres;polygon=@minimum_polygon_area_square_metres`; `undecided` = `undecided_count;objects=@counterparts` | `share` at least `minimum_contact_ratio`, no rounding |

The contact values measure one request per face for every value reading
it, with the rule's counterparts (every other object where the rule names
none); counterparts the selector cannot decide may add contact, so the
share runs up to the whole face and a shortfall is left open with `contact
ratio {share:lower4} is below required {minimum_contact_ratio:fixed4}, but
the counterpart selection is undecided for {undecided:least} object(s)
that could support the face`, while a pass stands. Before anything is
read, `skip_top_storey` and `skip_bottom_storey` each leave a face
unjudged (`unless`) where `storey_end;end=top|bottom;storeys=@storey_selector`
along the rule's traversal is 1: its storey is the highest or lowest of its
source by the storeys' `Elevation` attribute (`storeys cannot be ordered:
…` where one has none, and the face open where it reaches no storey or
several). A shortfall is graded (`grading`) by the share,
`contact_gap` and `relative` (the share over the minimum, derived): none of the face in contact
(`no contact`) is an error without a candidate near, informational
nearer than 0.1 m, an error farther than 0.5 m and a warning between; a
partial one (`contact ratio {share:lower4} below required
{minimum_contact_ratio:fixed4}`) informational above 0.9 of the minimum,
an error below 0.3, a warning between. The finding relates what the face
rests on (`related: share`). The declaration is refused per face after
`slab-contact declaration is invalid` (`prefixed`), in the capability's
order and words; the storey selector and the traversal are checked only
while a storey is left out (`when`), and without the contact service each
face is open (`contact service is not registered`).

### `counterpart-coverage`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `covering` = `counterpart_covering;by=@counterparts;measure=@measure;horizontal=@horizontal;vertical=@vertical;axis_tolerance=@axis_tolerance;frame=@infill_counterparts;infill_above=@infill_above` | `plan` (`measure` not `elevation`, `horizontal` at least 0), `height` (likewise with `vertical`) and `elevation` (`measure` `elevation`): each its `share`, `uncovered` and `whole` (`counterpart_uncovered_share`, `counterpart_uncovered`, `counterpart_whole` with the same arguments and `measure` the check's), the elevation also `framed` (`counterpart_infill`); `share` at most `lowest`, no rounding, graded |

`horizontal` and `vertical` are defaults taken `from` `tolerance` or the
check's own tolerance, `lowest` from the first threshold stated
(`info_above`, `warning_above`, `error_above`), `measure` defaults to
`plan_and_height` and `infill_above` to 0.5. The form's one value is read
first: the services the rule's checks need (`plan-area service is not
registered`, …), an element whose extent cannot be read (`…; its coverage
was not checked`) or whose footprint cannot be measured leave it open once
for both checks, as the capability did. A finding is graded by the first
band its share's upper end exceeds (`Exceeds`, error, warning, info) and
deviates from `lowest`; it reads `plan: {share:area} of the footprint
({uncovered:area} of {whole:area} m²) lies outside every counterpart grown
by {horizontal:si} m{overlaps}{graded}`, the height's and the elevation's
alike (the elevation naming the frame's infill, `{infill}`, where
`counterpart_infill` says it covers or may, its members `{framed:cited}`),
`overlaps` being `; no counterpart overlaps it` where `covering` is surely
zero and `graded` `; graded error by its upper bound, at least warning`
where the share's lower end reaches a milder band than its upper end. A
share straddling `lowest` leaves the check open with `…, which straddles
the threshold {lowest}{share:notes}`, the notes saying why it may be
covered more. The finding relates the counterparts surely covering part of
the element (`related: share`). The declaration keeps the capability's
checks, words and order: the tolerances' dimension (`quantity`) and
combination (`anyOf`, `exclusive`, `together`), the checks not both off
(`holds`), each threshold's kind, range (`holds`) and order (`exceeds`),
one threshold stated, `measure` (`choice`), an elevation with both
tolerances (`holds`), `axis_tolerance` (`angleBelow`) and the infill
(`requires`, `requiresValue`, `holds`).

### `effective-coverage`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `reaching` = `effective_reaching;sources=@sources;blockers=@blockers;mode=@mode;range=@range;touch_tolerance=@touch_tolerance;area_property=@area_property;access_path=@access_path;door_selector=@door_selector;opening_selector=@opening_selector;space_selector=@space_selector;capacity_property=@capacity_property;capacity_multiplier=@capacity_multiplier;capacity_multiplier_property=@capacity_multiplier_property` (unstated ones dropped) | coverage: `share` (`effective_share`) at least `minimum_ratio`, no rounding, over `area` and `covered`; capacity (`capacity_property` stated): `capacity` (`effective_capacity`, without an upper bound where `effective_unread` counts a contribution not read, `Derived::Open`) less `area` (`spare`, `Derived::Difference`) at least zero; missing values (likewise): each item of `effective_missing` a finding |

Every value names every parameter as the rule names it, so they share one
measurement per element. `reaching` is read first: missing services, an
element whose extent cannot be read or whose coverage cannot be measured
leave it open once, and one stating no area under `area_property` is the
finding `missing value: its {area_property} is not stated` (`reaching` is
then stated absent), checked no further, as the capability did. The
coverage finding reads `{share:area} of {named} ({covered:area} of
{area:area} m²) lies within the sources' effect areas ({mode} by
{range:si} m); required {bound:plain}{unreached}` (`named` the stated area
or the footprint, `unreached` `; no source reaches it` where `reaching` is
zero), relating the sources surely contributing (`related: share`); a share
straddling the minimum is open with `…, which straddles the bound
{bound:plain}{share:notes3}`, the first three notes saying why it may be
covered more. The capacity finding reads `capacity: {summed} is
{capacity:area} m² for {against} of {area:area} m²` (`summed` the sum's
words by the multiplier declared), relating the sources surely
contributing; one that cannot be decided `…, which cannot be
decided{summed:notes3}`. Each surely contributing source stating no
capacity or multiplier is a finding of its own, after the capacity's:
`missing value: {source}'s {property} is not stated`, relating the source
(`Decision::Items`). The declaration keeps the capability's checks, words
and order: the access declaration is the measured value's own argument
check (`arguments`, `AccessDeclaration::parse`), the modes' exclusions and
the capacity's combinations conditions (`holds`).

### `slab-stack-spacing`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `next` = `stack_distance;measure=top_to_top;slabs=@selection;ratio=@minimum_overlap_ratio`, `optional`: a slab none stacks above passes | per measure: `distance` within `<measure>_minimum` and `<measure>_maximum` (where one is stated); where `consistent` lists the measure, `distance` near `stack_prevailing;…;tolerance=@tolerance` (`optional`) within `tolerance` (1 mm by default) |

The values measure the stacks over the rule's own selection once per run,
its undecided objects possible next slabs up but never measured from:
which slab is next up, each consecutive pair, and each stack (slabs
connected through consecutive pairs) with the distance most of its pairs
share. A slab whose extent cannot be measured is open for its reason and
leaves every other open (`the vertical extent of … could not be
measured, …`); one whose next slab up cannot be decided is open once. A
distance's finding relates the next slab up (`related: distance`) and
reads `{label} to {distance:cited} is {distance:length}; required {bound}`,
a consistency finding `… which differs from the prevailing
{prevailing:length} in this stack`. The declaration is refused once for
the rule (`servicesPerObject`), in the capability's order and words; the
services missing leave each slab open.

### `recess-width`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `recesses;requirements=@requirements`: each recess's `width` at least its `required` width |

The measured `recesses`, handed the rule's rows, state each recess's
place, width and depth, the first row whose depth range holds it (`row`,
`null` where none does, undecided where its depth straddles a row's bound)
and the width that row requires (`required`, an interval over the depth).
A guard on `row` passes a recess no row holds and leaves one whose row is
undecided open (`{place} is {width:length} wide and {depth:length} deep;
which row applies is undecided`); the test judges the width against the
required interval, failing where it misses the interval's lower end and
passing where it reaches the upper (`…; row {row:count} requires at least
{required}`, and `…, undecided` where it straddles). The declaration is
`requirements` stated and every row read as the capability read it (the
list's argument check, `row 0 needs …`), refused once for the rule; the
plan-span service missing leaves each object open
(`servicesPerObject`), and a footprint the service cannot measure leaves
it open for the service's reason (`the recesses of … cannot be
measured: …`).

### `light-well`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `area` = `well_section_area;members=@member_path` | `Items` over `well_gaps;members=@member_path`: each gap at most `gap_tolerance_metres` (0 by default); `Items` over `well_requirements;members=@member_path;requirements=@requirements`: the section shared, its `area` and `width` at least what the row its height selects requires |

The section is read first, and measured only once the stack is, so a
well the path reaches no space from (`… reaches no space through …`),
whose spaces' extents or shared section cannot be measured is open once,
for that reason, as the capability left it. `well_gaps` lists
each pair of consecutive spaces, ordered by their bottoms, with the gap
from the lower one's top to the upper one's bottom (none below zero),
judged against the tolerance: `{above} starts {gap:length} above the top
of {below}, so the well is not contiguous`, or open with `the gap between
{below} and {above} is {gap:length}`. `well_requirements` is one item: the
section's `area` (`null` where the spaces surely share none, a finding,
`the {count:count} stacked spaces share no plan section, …`), its `width`
(`null` without one), the well's `height`, the row its height selects
(`row`, a guard: `null` judges nothing, undecided leaves the well open
with `which row applies to a well {height:length} high is undecided`) and
what it requires; each of area and width is its own finding (`the well's
section area is {area:area} m²; row {row:count} requires {bound:plain} m²
for a well {height:length} high`). Every finding relates the well's
spaces (`related: members`). The declaration keeps the capability's checks
and words: `member_path` and its steps, `requirements` and each row (the
list's argument check), the tolerance not negative.

### `centre-line-distance`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `centre_line_sides;walls=@wall_selector;centre_line=@centre_line;sides=@sides;reach=@reach;inset=@inset`: each side's `distance` none within reach, at least `minimum`, then at most `maximum` |

The measured sides are the capability's own reading (`wall_sides.rs`,
`centre_line_distance::line`): one item per side for `sides` `both`
(named `centre line to the left` or `… to the right` where a front is
known, `… beside the +first side` otherwise), or one for the nearer
(`centre line to the nearest wall`), each with `distance` (from every
wall that may lie there to the nearest wall surely there, or just past
`reach` without one; `null` where none may), `lower`, `sure` (the nearest
sure wall's own interval) and `wall`. Its tests: `null` is `{label}: no
wall nearby (none within {reach:length})`; where `minimum` is stated, a
distance below it is `{label}: too close: {sure:length} from {wall}, …`,
one above it is judged against `maximum` (`then`), and one straddling it
is judged against `maximum` with its pass open (`straddled`, an effect
on the pass); without it, the maximum alone. Too far reads `{far}`, a text
naming the sure wall or `no wall within the maximum …`, and an open side
`{undecided}`, a text naming the walls' interval or the least distance a
wall may lie at. Findings relate the sure wall. A side list that cannot be
measured (walls unmeasured, no long axis, no front) leaves the object open
once, `centre line: {why}`. The declaration keeps the capability's checks,
words and order (`finiteLength` for its lengths); without the plan-span
service the rule is open as a whole.

### `component-visibility`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `sight_view;targets=@targets;blockers=@blockers;eye_height=@eye_height;radius=@radius`: `visible` at least `minimum` (1 by default) under `mode` `at-least`, at most 0 under `none` |

The view is the capability's own search (`View::of`): the line-of-sight
service asked about every target, selected or undecided, from the eye
over the component's exact centre, the undecided blockers re-asked
without. Its one item counts the targets in view from those surely in view
to every one that may be (`visible`), states `sure`, `hidden` and their
objects (`seen`, `found`), and words the eye (`within`) and the first
three undecided targets (`undecided`). Under `at-least` a shortfall reads
`{sure:count} target(s) {within} are in view; required at least
{minimum:count}` and `; {hidden:count} hidden` where any is, relating the
targets in view and hidden; under `none` a target in view reads `… are in
view, none allowed: {seen}`. Services missing and an eye that is not a
point leave each component open, as the view reports them. The
declaration keeps the capability's checks, words and order (`holds` for a
positive minimum).

### `exit-separation`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `exit_separation;exit_path=@exit_path;exit_selector=@exit_selector;…;minimum_exits=@minimum_exits` (every parameter under its own name, unstated ones dropped): the count item's `exits` at least `minimum_exits`; the separation item's `separation` at least its `required` |

The list is the capability's own reading (`Reached`, `Measured`): the
exits `exit_path` reaches among the objects `exit_selector` picks, those it
leaves undecided possible exits, the longest plan diagonal, the share the
flag selects and every pair of the sure exits. Where the rule declares
`minimum_exits`, a first item counts the exits from the sure to the
possible (`exits`): fewer possible than required is a finding (`has
{possible:count} exit(s) via {relation}; at least {minimum_exits:count}
required`), fewer sure is open, and then nothing else is judged. With two
exits that may be, a second item states the separation of the pairs, the
greatest for `pairs` `any` and the least for `all` (`null` where none is
measured, undecided with fewer than two sure exits), against `required`,
the share of the diagonal as an interval: where the flag is unknown it
spans both shares, so a verdict stands only where both agree (D10); never a
default. The range judge fails a separation surely below the interval's
lower end and passes one at least its upper end. A pass under `all`, or a
failure under `any`, that pairs not measured or undecided exits may change
is open (`undecided_pass`, `undecided_fail`, through an effect and the
`otherwise` test), and a space surely short of exits keeps its count
finding alone: what the separation leaves open passes there (`short`, the
`straddled` and `otherwise` tests and the `null` effect). Findings read
`{failed}; {requirement}` (`exits d1 and d2 are 2 m apart between closest
points; required at least 11.1803 m (0.5 × the longest plan diagonal of
22.3607 m)`, or `no two of its 3 exits are far enough apart: …`) and relate
the pairs too close and the objects the flag was read on; an open space
reads `{open} ({requirement})`. A path that cannot be read, a missing
service, a diagonal that cannot be measured and a flag holder outside the
project refuse the list for their reason. The declaration is the
capability's own (`arguments`, `exit_separation::check_arguments`), refused
once for the rule.
### `name-sequence`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `name_sequence;member_selector=@member_selector;name=@name;order=@order;first=@first;increment=@increment;order_fallback=@order_fallback;…` (the traversal parameters under their own names, unstated ones dropped), each item judged on its `member` (`at`) |

The list is the capability's own reading (`name_sequence::members`): the
members the bound member selection picks that the traversal reaches from
the anchor (or every one of its source but the anchor), each ordered by its
`order` value or, with `order_fallback`, its placement height, ties by
name. Each item states the member's name (`shown`), whether it is set and a
whole number, its number (`value`), the number of the member below it in
the sequence (`previous`, `null` for the first), the number expected of it
(`expected`: `first` (1 by default) for the first, the previous plus
`increment` after) and the member below (`below`). The sequence runs on
from the last member whose number counts (stated, whole, not below
`first`), as the capability read it. The tests, each in the last one's
place where it passes (`then`): `{name} is not set`, `{name} {shown} is
not a whole number`, `{name} {value:count} is below the start
{first:count}`, and the number `expected`, worded `{name} of the first
member is …; expected …`, `… is not above {previous:count}, the member below
it` or `… does not follow {previous:count}; expected {expected:count}`
(`broken`, by `previous` and `above`), relating the member below. A member
selection with an undecided object, a member without an order value and a
member the object-frame service cannot place refuse the list, leaving the
anchor open for their reason. The declaration is the capability's own
(`arguments`, `name_sequence::check_arguments`), refused once for the rule.

### `numbering-consistency`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `numbering;property=@property;pattern=@pattern;…;selection=@selection` (every parameter under its own name, unstated ones dropped, the rule's own selection bound) |

The list is the capability's own reading (`Collected`): the number the
pattern (`MeasuredParameterKind::Pattern`, bound as the rule states it,
never trimmed) reads from each selected object, and the scope it lies in,
read once per rule for the whole selection. An object's items are: why its
number or scope cannot be read (`number`, refused for that reason, as the
capability left it open); with `prefix_length`, where the scope holds at
least two prefixes, its prefix's `lead` (how many more objects share it
than any other prefix, less the objects of the scope not read), at least 1
or a finding worded by `departs` (`… does not start with 2, the prefix of 2
other object(s)`, or `…: its scope mixes the prefixes 2 (2), 3 (2)`), a
number of fewer digits open (`… has fewer than 1 digit(s), so it has no
prefix`); with `gap_free`, its number's `step` from the next lower number
of the scope, at most 1 or a finding (`{property} {shown} follows {below};
{missing}`), open where an object not read may fill the gap (`fillable`).
Findings relate the objects the prefix or step is judged against. The
declaration is the capability's own (`arguments`,
`numbering_consistency::check_arguments`), refused once for the rule.

### `wall-spacing`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `wall_spacing;members=@members;member_path=@member_path;…;uncovered_above=@uncovered_above` (every parameter under its own name, unstated ones dropped) |

The list is the capability's own reading (`Storey::pairs`, `Bands`,
`uncovered`): the members the path reaches among those the bound member
selection picks, each pair parallel and facing, the bands between pairs at
most `maximum` apart and each footprint's area outside them. Its items:
each pair surely parallel, facing and selected, with its plan distance,
at least `minimum` or a finding (`{pair:and} are parallel and {apart} m
apart in plan; at least {minimum:si} m required`), a distance straddling
the minimum passing here, since the list words every doubt once: why some
pair may stand closer than the minimum (`minimum spacing: whether every
parallel pair stands … apart is unknown: …`), open; and each footprint's
area outside every band, at most `uncovered_above` (an area, bound in
square metres) or a finding (`{what}; at most {uncovered_above:si} m²
allowed`), open where it straddles (`{what}, which straddles … m²{unknown}`)
or cannot be measured, for its reason (`MemberValue::Refused`). Missing
services and a path that cannot be read refuse the list for each storey.
The declaration is the capability's own (`arguments`,
`wall_spacing::check_arguments`), refused once for the rule.

### `parking-bay`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `parking_bay;min_width=@min_width;…;neighbour_reach=@neighbour_reach;selection=@selection` (every parameter under its own name, unstated ones dropped, the rule's bays bound) |

The list is the capability's own reading (`Bay::steps`): the aisles,
neighbouring bays and obstacles near each bay from one plan broad phase per
rule over its bays (`Nearby`), the bay's least-area rectangle and vertical
extent, the obstacles counted within it and at its ends and sides, its
orientation to an aisle, and in filter mode the states it may be in and
whether its size bounds apply. Its items, in the capability's order: a
count against what is allowed (`count` from sure to possible at most
`allowed`: a finding `{found_words}`, open `{open_words}`; an obstacle
within the bay is one, none allowed); a size within its bounds (`size`
within `low` and `high`, items of the rule's bounds: `{measured};
{bound:plain} m{suffix}`, open where it straddles, and where the filters
may leave the bay out a shortfall is open, `…, if the bound applies to it:
{applies_why}` (`doubtful`); a bay the filters surely leave out has no size
item); and a search's own answer, the orientation to an aisle (`found`, a
finding worded `{message}`, open for its reason). Missing services and a
broad phase that cannot be run refuse the list for each bay. The
declaration is the capability's own (`arguments`,
`parking_bay::check_arguments`), refused once for the rule; the strings it
binds untrimmed (`pattern`).

### `distance`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `distance_items;counterparts=@counterparts;…;selection=@selection` (every parameter under its own name, unstated ones dropped, the rule's subjects bound), joined by `; `; of the project, `Items` over `distance_open` with the same arguments, each item's outcome on its `object` |

The list is the capability's own reading (`Pairs::among`, then `verdicts`
for each subject): one broad phase per rule over the bound subjects and
counterparts (or their door swings), the counterparts each subject shares a
container with and stands at the heights of, measured in the declared
projection, and what keeping them apart and having them within reach come
to. A subject's one item holds `apart` (the nearest counterpart surely
closer than `minimum_metres`, at least the minimum, graded), `reach` (in
`nearest`, the nearest counterpart where none surely lies within, at most
`maximum_metres`, graded), `none_within` (no counterpart near at all, a
finding) and `count` (in `at_least`, from the counterparts surely within
the range to every one that may be, at least `count`, ungraded), each
worded as the capability worded it (`{apart_words}`, `{reach_words}`,
`{count_words}`) and relating what it named; a check left open is its
number refused for why. Its failing checks are one finding, graded by the
worse deviation where each states one; where none fails, the first left
open (`joined`). The objects the selections and the broad phase leave open
beyond the subjects (an undecided or unreadable counterpart) are the
project's `distance_open`, each open for its own reason. The declaration
is the capability's own (`arguments`, `distance::check_arguments`),
refused for each subject; `elevation_overlap` `overlapping` needs the
vertical-extent service, refused once for the rule (`services` with
`only` and `whole`). Computed bounds run per object, as for any template
(`object_parameters`).

### `containment`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `judged` = 1 | `Items` over `containment_items;counterparts=@counterparts;…;selection=@selection` (every parameter under its own name, unstated ones dropped, the rule's inner elements bound), joined by `; `; of the project, `Items` over `containment_counts` with the same arguments, each item's outcome on its `object` |

The lists are the capability's own reading (`assess`): one broad phase per
rule over the bound inner and outer elements, each inner element placed
in the outer elements (alone, or with `combine_adjacent` in adjacent ones
taken together), each cover band's face distance to an outer element it
lies in, and the inner elements each outer element surely and possibly
holds. An inner element's items, ordered as the capability reported what
it left open: whether it lies in none (`orphan`, a finding worded
`{orphan_words}`, or open for why), each band's distance within its
bounds (`distance` at least `low`, then at most `high`, items of the
band: `{below_words}`, `{above_words}`, a straddle `{straddle_words}`,
relating the outer element) and why something of it is not checked
(`open`). The project's items: each outer element's count (`count`, from
sure to possible, at least `minimum_count`, then at most `maximum_count`:
`{fewer_words}`, `{more_words}`, open `{between_words}`, relating what it
surely holds), where the rule bounds counts, and each object the
selections and the broad phase leave open beyond the inner elements. The
declaration is the capability's own (`arguments`,
`containment::check_arguments`), refused for each inner element.

### `coordinate-consistency`

| Form | When | Once | Scopes | Decision |
| --- | --- | --- | --- | --- |
| one | always | `reference` = `coordinate_reference;reference=@reference`, required | every source (`every`) | `Joined` over `coordinate_differences;reference=@reference;length=@length_tolerance;angle=@angle_tolerance;scale=@scale_tolerance;require_map=@require_map_conversion` |

The reference source is read once per rule, of the project: fewer than
two sources, a discipline no source or several declare, sources declaring
none and a run without declared disciplines leave the rule open, after
`coordinate-consistency:`, and nothing else is judged. Every source of the
session, one holding no object too, is then judged by the differences
its coordinate system shows against the reference's
([Coordinate systems](./derived.md#coordinate-systems)), in
`compare_coordinate_systems`' words and order, the georeference last:
a finding reads `` `{source}` does not share the coordinate system of
`{reference:source}`: {found} `` (the words joined by `; `), the
reference's own missing map conversion `` `{source}`, the reference,
{found} `` (`Condition::Scope`), and a source with nothing found but a
statement it cannot compare is open, ``coordinate-consistency:
`{source}` against `{reference:source}`: {open}``, as not recorded where
only the georeference is unknown. A system that cannot be read (the
reference's, for every other source) leaves the source open after
`coordinate-consistency:`. The declaration reads the three tolerances'
kinds, then checks each is not negative (`the length tolerance must be
finite and not negative`), then reads `reference` and
`require_map_conversion`, in the capability's order; the defaults are a
millimetre, a hundredth of a degree and an identical scale. Without the
coordinate-system service the rule is open before anything is read.

### `external-wall-validation`

| Form | When | Once | Scopes | Values | Checks |
| --- | --- | --- | --- | --- | --- |
| one | always | `all_spaces` = `envelope_size;derivation=all-spaces;bounding=@bounding_selector`, where `derivations` lists `all-spaces`; `gross_area_groups` = `envelope_size;derivation=gross-area-groups;groups=@gross_area_group_selector;group_path=@gross_area_group_path`, where it lists `gross-area-groups` | each source holding a selected wall (`occupied`), where either derivation was measured | `declared` = `external_declarations;derivations=@derivations;…;objects=@selection`, at least `one` | per derivation measured: `on_envelope` at least `declared_external` (`declared external but not on the {derivation} envelope`); at most it, unless the source declares nothing (`on the {derivation} envelope but not declared external`, quiet); with both: `on_envelope` of one at least and at most the other's, unless the object bounds either (`bounds_envelope`) |

Refusals wait for the selection (`selected`): a rule selecting nothing
says nothing, and otherwise the declaration is refused once after
`external-wall-validation:` in the capability's order and words
(`derivations` required and listing one, each option as stated and once
(`listed`), the selectors' and path's kinds, the group selector and path
together and the path valid, then what each listed derivation needs, in the
list's order (`listedNeeds`)); without the envelope-membership service the
rule is open (`envelope-membership service is not registered`). Each
listed derivation is measured once, in the list's order: one that cannot
be derived (an undecided or empty bounding selection, groups without
members along the path, a service refusing it) leaves the rule open with
`{derivation} envelope: {why}` and is never read again. Each source
holding a selected wall is then judged: none declared external is an
error on the source (`no selected object is declared external: the model
declares no envelope`), relating nothing, and walls whose declaration is
unknown leave it open (`… but {declared:upper0} state neither external
nor internal or could not be measured`). Each selected wall is judged per
derivation, each failing check a warning of its own: a wall whose
declaration is unknown is open once per derivation (`not compared with the
{derivation} envelope: …`), its other checks quiet.

### `space-boundary-coverage`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | `off` = `boundary_coverage_off;plane=@plane_tolerance`, `zero` = 0 | share (`minimum_covered_share` stated): `share` (`boundary_coverage_share`) at least the minimum, worded over `surface` and `uncovered`; uncovered (`maximum_uncovered_area` stated): `uncovered` (`boundary_coverage_uncovered`) at most the maximum; overlap (`maximum_overlap_area` stated): `overlap` (`boundary_coverage_overlap`) at most the maximum; no rounding |

`plane_tolerance` defaults to 0 m. The values read one boundary-coverage
request per space and tolerance for the run (`BoundaryMeasures`); a space
the service cannot measure is open once, `space-boundary coverage: …`, for
the reason its refusal gives. The form's own decision is `off` at most
zero: a boundary off the body's surface is always a finding, `space
boundary {off:noted} lies on no face of the space's body, so it covers
nothing`, relating the elements it bounds against (`related: off`). The
checks read `declared boundaries cover {share:percent} of the
{surface:m2} surface, leaving {uncovered:m2} uncovered; at least
{minimum_covered_share:percent} required`, `declared boundaries leave
{uncovered:m2} of the {surface:m2} surface uncovered; at most
{maximum_uncovered_area:m2} allowed` and `declared boundaries overlap over
{overlap:m2} of the surface{between}; at most {maximum_overlap_area:m2}
allowed`, `between` naming the pairs that surely overlap (` (boundaries
b1 and b2)`, `Condition::Noted`) and the finding relating their elements
(`related: overlap`); a straddling check ends `, which straddles the
required …` or `the allowed …`. Everything one space leaves open is one
outcome, joined by `; ` (`joined`). The declaration keeps the
capability's checks, words and order; without the service the rule is
open (`space-boundary coverage service is not registered`).

### `horizontal-guard`

| Form | When | Values | Checks |
| --- | --- | --- | --- |
| one | always | once, required: `surfaces` = `guard_surfaces;surfaces=@selection;…`, of the project | edges: each item of `guard_edges;surfaces=@selection;…` judged by its first defect, merged per defect |

Every value names the search as the rule states it (`barrier_gap`,
`platform_gap`, `landing_gap`, `landing_width`, `climb_distance`,
`climb_side`, `from_curb`, `climb_height` and the role selectors, an
unstated one dropped) and the rule's own selection, so the rule's
surfaces are measured in one request for the run. `guard_surfaces` is
read once per rule (`once`, `{why}` its refusal): unusable thresholds (`horizontal-guard thresholds do
not define a usable search`), a role selection leaving objects undecided
(`` horizontal-guard: `barrier_selector` cannot be decided for 2
object(s) ``) or a refused request leave the rule open as a whole. A
surface without a measured edge is open (`no edge was measured for this
walking surface; it has no measurable body`). Each edge is judged by a
chain of tests (`then` where one passes, `otherwise` where it fails,
`unmet` nulls failing, lengths raised by a micrometre against their
bounds as the capability compared them): `guarded_height` at least the
barrier height, then `climbable_height` at most the climbable height
(`barrier_too_low_due_to_climbable_object`, relating `climbable`);
otherwise `barrier_share` above one half: `tallest_barrier` too low
(`barrier_too_low`, or, `tallest_top` reaching the height,
`barrier_too_low_due_to_curb`), else `partial_height` reaching it
(`hole_in_barrier`, relating `tallest`); otherwise, and where neither
applies, `landing_fall` at most the fall height, else the nearest landing
too far (`landing_too_far_away`), too low, too small or too few
(`insufficient_landings`), relating `nearest`, else `missing_barrier`.
Findings of one defect on a surface are one (`merged`), graded error
whatever the rule's severity. The declaration is refused once, after
selecting (`selected`): every threshold a finite number of at least zero
and the curb flag stated (`horizontal-guard declaration is missing or not
realisable`, naming the capability itself), then each role selector's
kind after `horizontal-guard: `; a rule selecting nothing judges nothing. Without the
service the rule is open (`guard service is not registered`).

### `corridor-end-openings`

One form, judged by one check over the measured list
`corridor_end_openings;path=@opening_path;openings=@opening_selector;depth=@wall_depth;facing=@facing`:
each opening the corridor reaches, searched against the walls its ends
run into (the plan-span service's corridor ends) as the capability
searched them: `sits` in one (true, false, or undecided with why), the
`walls` it sits in, and whether the selection picks it (`picked`, its
reason `unpicked`). The list names the opening as each item's `subject`,
so every outcome is on the opening: a selected opening sitting in an end
wall is a finding relating the corridor (`sits in the end wall of
corridor {corridor}: {walls}`), one whose standing is undecided is open,
and one the selection cannot decide is open only where it sits in one. A
corridor whose ends cannot be measured, or without the plan-span
service, is open on the corridor with the list's refusal and reason. The
declaration checks the path, the selector and both margins (`nonNegative`),
in the capability's order and words; an unstated margin is the search's
default (0.5 m deep, 0.1 m facing).

### `space-connection`

One form, judged by its check over the measured list
`space_connections;connections=@connections;access_path=@access_path;door_selector=@door_selector;opening_selector=@opening_selector;space_selector=@space_selector`:
one item per requirement of each row whose `from` picks the space, its
`access` and then its `exit` (an `allowed` one is no item), each stating
whether the space is surely linked as the row asks (`linked`: to a space
`to` surely picks through an element surely of the row's type, or to the
outside; undecided where an element of undecided type, one whose spaces
cannot be read, or a linked space `to` cannot decide could change it), the
row's words (`row`, `kind`, `via`), the sure links (`links`) and the
objects a finding relates (`related`: the links' spaces and elements, or
the elements reaching the space).

| Test | When | Judgement |
| --- | --- | --- |
| required access | `access`, `required` | `linked` false is a finding (`has no direct access through a {kind} to a space {row} requires (via {via})`) |
| forbidden access | `access`, not `required` | `linked` true is a finding (`has direct access to {links}, which {row} forbids for a {kind}`) |
| required exit | not `access`, `required` | `linked` false is a finding (`has no {kind} directly to the outside, which {row} requires`) |
| forbidden exit | neither | `linked` true is a finding (`opens directly to the outside through {links}, which {row} forbids for a {kind}`) |

An undecided `linked` leaves the row open (`space-connection {row}: {why}`),
each row its own outcome as the capability reported them, and a space
whose rows cannot be told (a row's `from` that cannot decide it) is open
once with the list's refusal. The declaration is the list's own argument
check (`Check::Arguments`): the access path and selectors, then the rows,
in the capability's order and words.

### `opening-spaces`

| Form | When | Scopes | Values | Decision | Checks |
| --- | --- | --- | --- | --- | --- |
| one | always | each source where the rule selects an element (or may), or where `host_selector` picks a wall (`reached`) | `declared` = `any_external;host_selector=@host_selector;external_property=@external_property` (whether any of its walls declares itself external: 1, 0, or between), `walls` = `host_walls;…`, `unknown` = `undeclared_hosts;…`, `maybe` = `possible_hosts;…` | `declared` at least `one`: `{found}` (`source … has no host wall, so none is external` where `walls` is zero, `none of the {walls:upper0} host wall(s) in source … is declared external` otherwise), relating the walls; open (`… but {unknown:upper0} wall(s) do not declare … and {maybe:upper0} more may be walls`) | each element by `connected_spaces;host_path=@host_path;…;space_path=@space_path;space_selector=@space_selector` |

The element's one item states its hosts' exposure and its spaces: `count`
at most `expected` and `possible` at least it (`{relates}; in {wall}
({hosts}) it needs {requirement}`), then every space decided (`known`,
open with its own words), then, for the derived adjacency, its spaces on
the faces the exposure needs (`sides`, a finding worded `{placed}; …`),
each test only where the one before passes (`then`). An element whose
hosts are undecided, reached by none, declare nothing or disagree, or
whose spaces cannot be read or placed on a face, is open with the list's
refusal, worded as the capability left it. The declaration is the list's
own argument check (`Check::Arguments`), in the capability's order and
words.

### `door-swing`

One form, judged by its checks, both over the measured list
`swing_spaces;path=@space_path;towards=@swing_into;not_towards=@swing_not_into`:
the spaces the door opens onto that either selector may pick, whether each
picks it (`towards`, `not_towards`: true, false, or undecided), each probed
once (`into`, and `away`, undecided where neither probe lies in it).

| Check | When | Judgement |
| --- | --- | --- |
| not into | `swing_not_into` | each space's `into` a finding (`swings into {space}, which `swing_not_into` forbids`, relating it), open where the selection may pick it or the probes cannot place it |
| into | `swing_into` | `Any`: a picked space swung into passes; otherwise the first space left open (the probes cannot place it, it may not be picked, or neither probe lies in it) opens the check; otherwise the picked spaces surely swung away from are one finding (`swings away from {failing}, …`) |

A guard (`unless`, `guard: true`) reads `hinged_leaves` first: an object
without a hinged leaf, or whose leaves cannot be read, is open with
`door-swing: {why}` before its spaces are listed, as the capability
refused it first. Both checks leave the door open once (`once`), as the
capability reported one doubt per door, and a door whose spaces cannot be
reached is open once with the list's refusal. The
services are `object-frame` and `free-space`, checked before anything is
selected. The declaration checks `space_path`, both selectors' kinds,
that one is stated, and the path, in the capability's order and words.

### `opening-area`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `gross`, `net` = the stated `{gross_area}` and `{net_area}`, areas or absent; `agrees` = either stated implies `abs(opening_area;… − (gross − net))` at most `area_tolerance` and the rounding allowance; `voided` = `opening_area;…`, `expected` = `gross − net`, read only for a finding | `Holds` over `agrees` |

A side area stated of another kind is open as the capability worded it
(`` `{gross_area}` is 15 m, not an area ``), one that cannot be read too
(`` `{gross_area}`: {why} ``, the value's `refused`); neither stated
holds without measuring, and one stated alone leaves the difference
`null`, the wall open with `it states only one of `…` and `…`` (D12's
decision). The declaration is `empty-host`'s, then both side areas
required and the tolerance. The finding reads `{openings}, but its gross
side area {gross:m2} less its net side area {net:m2} is {expected:m2};
they must agree within {area_tolerance:m2}`, `openings` being `its
openings ({voided:cited_ids}) cover {voided:m2} of its face` where
`opening_area` cites an opening and `it has no openings` otherwise, and
relates every opening reached.

### `empty-host`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `voided` = `opening_area;…;cites=counted`; `kept` = `voided` above 0 m² implies `voided` below `middle_face_area;…` less `area_tolerance` and the rounding allowance; `face` = `middle_face_area;…`, read only for a finding | `Holds` over `kept` |

The opening values name the rule's declaration as arguments
(`path=@opening_path;length_axis=@length_axis;height_axis=@height_axis;minimum=@minimum_opening_area;openings=@opening_selector`,
an unstated minimum or selector dropped), citing the openings that take
area from the middle plane (`cites=counted`): an area above zero is an
opening there, since only an opening of area takes it. The rounding
allowance is the capability's, a nanometre times one square metre plus
the face and the openings' area. A host with no opening on its middle
plane holds, its face never measured. The declaration checks the path,
the selector's kind, both axes (stated, then as the measurement reads
them: `Check::Arguments`, the axes' options, that they differ, and the
minimum area) and the tolerance (`nonNegativeQuantity`), in the
capability's order and words; the tolerance defaults to 0 m². The finding
reads `host is empty: its openings ({voided:cited_ids}) void {voided:m2} of
its {face:m2} face` and relates the counted openings.

### `keyed-limit`

One form, judged by its checks (`Holds` over the literal `judged`). The
row comes first, over the measured list `limit_row;limits=@limits;key_1=@key_1;key_1_path=@key_1_path;…;pair_key=@pair_key;case_sensitive=@case_sensitive`:
one item, the most specific row the object's keys select (`listed`, its
index `row`, the keys as a message describes them `keys`, the objects a
path key was read on `related`). No matching row is a finding (`no limit
defined for {keys}`); keys that cannot decide the row (a path key whose
objects disagree or reach none is unknown, never no row: D16) or rows
tying leave the object open with the list's refusal. Then the measured
list `limited_values` (the row's arguments and every parameter a quantity
reads) states what the row bounds with its `minimum` and `maximum`, one
check per form of `quantity`:

| Check | When `quantity` is | Items | Judgement |
| --- | --- | --- | --- |
| value | `plan-area`, `member-plan-area`, `property`, `measured`, `clear-width`, `clear-height`, `glazing-ratio` | one value of the object (`what`, `unit`) | a range against the row's bounds, graded: `{what} is {value:area}{unit}; required {bound:plain}{unit} (limit row {row:fixed0}: {keys})`, straddling it open |
| sill | `sill-height` | the sill height above each floor beside a window, each citing that floor's extent | `combined`: every failing floor in one finding, `(limit row …)` appended; any failing floor a finding beside an unknown one (D15) |
| step | `threshold-step` | the step onto each floor a side of a door may step onto (`side`, `sure`) | `combined` with `alternatives` per side: a side fails where a floor it surely steps onto fails, or every floor it may |

The list reads displayed values as the capability did: a clear width or
height, glazing ratio or measured value snapped to the precision it is
shown at before it is judged. A member area with members the selection
cannot decide is known only from below, an undecided value. The list is
read with the run's resolver, so a key a classification derives is read
as derived. The declaration is the list's own argument check
(`Check::Arguments`): the keys, the rows, then the quantity, in the
capability's order and words.

### `opening-zone`

One form, judged by one check over the measured list
`zone_checks;host_path=@host_path;host_selector=@host_selector;…;support_clearance=@support_clearance;openings=@selection`
(every parameter by its own name, and the openings the rule selects, so
each opening's neighbours are the capability's): what the capability
checks of the opening in each host its path reaches, in its order, host by
host (`check` names each item), every opening placed once per run.

| Item | Fields | Judgement |
| --- | --- | --- |
| `placement` | `placed`, `inside` (`outside`), `end`, `edge` (`edge_words`), `bottom`, `top`, `far_open` | `placed` undecided is open for the item's `reason` (a host or body that cannot be read, an area that may be below the minimum); `inside` false a finding (`opening lies partly outside its host {host}: {outside}`), undecided open; `end` at least `end_distance`, `edge` at least `edge_distance` (0 m with `zone` `web`) and `bottom`, `top` at most `edge_distance_maximum`, each within a nanometre and graded |
| `zones` | `zone`, `needed`, `words` | the side the opening misses most of the zone it misses least at least its inset, graded; `null` where it lies in a zone, undecided where it may |
| `dimension` | `distance`, `minimum`, `maximum`, `slack` | the row's range within its tolerance, graded (`{words}; {required} ({label})`), its doubt `{open}` |
| `support` | `fails`, `message` | a support surely missing the requirement a finding, one that may open, the supports that cannot be found open for the item's `reason` |
| `spacing` | `spacing` | the least clear distance to an opening surely too close at least `opening_spacing`, graded |

A distance measured to a free outline from the box the opening may lie in
is known only from below, up to infinity: it can find an opening but never
pass one. Both far edges known only from below are left open in one
outcome (`far_open`), as the capability reported them. Items come in the
capability's order, so an opening left open for several reasons is open for
the first as it was. Every finding of a placement relates its host and,
where it is surely too close to other openings, those openings too, as the
capability related them. An opening whose hosts cannot be told is open with
the list's refusal. The declaration is the list's own argument check
(`Check::Arguments`), in the capability's order and words.

## Binding and running a rule

The rules crate's `templates` module runs a template
(`templates::run`, behind the capability's `RuleCapability::evaluate`):

1. **Bind.** The declaration checks run over the rule's parameters. Each
   stated parameter becomes a constant, a quantity converted to coherent
   SI units exactly as the capability converted it, then the defaults
   apply and the form is chosen. Every slot in a string field of the
   form's expressions (`{axis}`, `{target_property.set}`,
   `{target_property.name}`) is filled and every `parameter` read of a
   constant folded into a literal. The result is the rule's plan: the same
   expressions an equivalent hand-written `expression` rule would hold.
   Binding is a pure function of the template and the rule's parameters,
   so each capability keeps the plans it bound (`Plans`, at most 64, by
   the rule's parameters) and a rule binds once, however many runs it is
   in (see [Template performance](./performance.md)).
2. **Check the services**, and leave the rule open without them.
3. **Select** the objects as every capability does (`select_objects`).
4. **Read the values** of each object through the same leaves an
   `expression` rule reads (`ObjectLeaves`): stated properties through
   property resolution and concept binding, measured values through the
   registered providers. A value that is one plain property read is read
   straight from the leaf (spending the same evaluation budget); any other
   expression goes through the shared evaluator. A value that is one
   measured read is measured for a chunk of selected objects at once
   through the run's `MeasuredValues`, the same value and evidence the
   run's resolver answers, and each object takes its own from the leaf.
   The first value that cannot be read leaves the object not evaluated,
   for the reason its leaf gives and worded as the measured value refused
   it (`{why}`); a `null` value is a missing-information finding
   (`absent`); a value not of the expected kind is invalid evidence
   (`mismatch`).
5. **Decide** and word the outcome with the form's messages.

### Messages

A message is a template of placeholders: a text's name, a parameter's
name (as stated; a property reference as `set.name`), a value with a
format (`{extent:length}`: `0.3 m`, or `between 0.48 m and 0.52 m`;
`{area:area}`: `26` or `between 24 and 26`, rounded to 1e-4 as the area
capabilities showed areas; `{target:stated}`: the value as the source
states it; `{minimum:fixed3}`: a constant, or a value known as one point,
with three decimals; `{length:upper3}`: a value's upper end, or a
constant, with three decimals; `{voided:m2}`: square metres to six decimals, a
value's midpoint or a constant, `15 m²`; `{count:cited_ids}`: the local ids of
the objects a value's measured reads cite, `o1, o2`), `{bound}` (the bound a `Within` failed or straddled, a length:
`at least 0.26 m`), `{bound:plain}` (the bound as declared: `at least 6`),
`{why}`, an anchor's `{undecided}` and `{relation}`, and a value's lower
end to two decimals (`{numerator:least2}`: `0.66`), a share's upper
end, at most 1, to four decimals (`{share:share}`: `0.4`), and a number
with a fixed number of decimals: a value known as one point or a
constant (`{minimum:fixed4}`: `0.5000`), or a value's lower or upper end
(`{share:lower4}`, `{length:upper3}`, `{declared:upper0}`), the sources a
value's measured reads cite (`{reference:source}`: the reference source),
a `Joined` form's `{found}` and `{open}`, a scope's `{source}`, and the objects a value's measured
reads were measured against (`{distance:cited}`: the next slab up, or
nothing where they cite none), what they noted, each after `; `
(`{share:notes}`: `; 1 counterpart(s) have no readable extent, so they may
cover it`, or nothing; `{share:notes3}` the first three; `{overlap:noted}`
the notes alone, joined by `; `), an area with its unit to the micrometre
squared and a share as a percentage to two decimals, a value's interval
as `between … and …` where its ends show differently, or a constant
(`{uncovered:m2}`: `7.5 m²`; `{share:percent}`: `87.29%`), a share a hundredfold (`{share:hundred3}`: the lower end, `12.500`; `{maximum:hundred}`: a constant or point as Rust shows it, `5`; items read them too, and `fixedN`, `lowerN`), and a constant's number in coherent SI units as
Rust shows it (`{horizontal:si}`: `0.02`, a tolerance as the capability
wrote it before its unit); a `Compare`
form also reads `{target}` (the stated target as the rule declares it,
after a space: ` 250 mm`, `` `F30` ``, ` [F30, F90]`) and
`{tolerance:suffix}` (` (within tolerance 0.01)`, nothing when exact).
A `Within` form also reads `{required}` (the declared range as a
requirement reads it: `between 1 and 3`, `at least 1`, `at most 3`) and
`{required:exactly}` (the same, `exactly 2` for equal bounds), and any
form `{count:least}` (a value's lower end, what surely counts: `2`).
Lengths are shown rounded to the micrometre, as the capabilities showed
them.

## Expand and fork

**Expand.** The catalogue's entry of a templated capability carries
`template`: the template itself (declaration, services, texts, forms with
their values, decisions and messages) and `requirements`, each form as one
expression (`Form::requirement`: the decision's expression form with every
value inlined) with its [block tree](./block-editors.md). Its expressions
still hold the slots and `parameter` reads; a block editor shows them as
written.

**Fork.** `axioval_rules::templates::fork(capability, rule)` copies a rule
bound to a template into the `expression` rule it composes
(`Fork::CAPABILITY`, `Fork::parameters`): the form's requirement with the
rule's parameters bound in, every measured read canonical (a `measured.`
block with its parameters as fields). It is the starting point for a
stricter or extended rule. The rule's
parameters its measured values name (`@door_selector`) travel with it
(`Fork::carried`, in `Fork::parameters`), so it binds them as the template
does; its definition declares them as authored parameters, a selector
among them. A form's checks fork into an `and` with its decision. The fork reads the same values through the same
evaluator and reaches the template's verdicts on every fixture
(`the_forked_rule_reaches_the_templates_verdicts`), with two differences
that are its own: its findings are worded as an `expression` rule's, and
its bounds are decided by the evaluator's sound interval arithmetic, which
may leave open a value within a unit in the last place of a bound's
rounding allowance that the judge decides.

A form judging members forks into aggregates along the rule's traversal:
the `path`, or the `relationship` in its `direction`, filtered by the
member selector. Members everywhere in an anchor's source, a followed
chain and skipped absent ends have no aggregate path, and such a rule is
not forked (`ForkError::Inexpressible`, naming why). An aggregate counts
an undecided member as possibly there, where the template leaves the
anchor open unless it already exceeds its maximum (D7): the fork's
verdict is sound but may decide an anchor the template leaves open. Under
`widen` the two agree. An unstated selector read as every object forks
into an unfiltered aggregate (`Selector::All`); shared ends have no
aggregate form, and such a rule is not forked.

A form whose decision only reads a value (a `Within` without a bound,
the checks deciding) requires that value stated (`isDefined`), unless it is
optional: a `null` is the template's finding. Its fork does too, and leaves
out every check that does not apply to the rule.

A form with a `scope` expands into its decision over the aggregate of the
rule's `selection`, as the catalogue shows it, but is never forked
(`ForkError::Inexpressible`): an `expression` rule judges objects, never
a source or the project as a whole. Neither is a form reading values once
per rule (an expression rule reads its values per object, and none whose
refusal leaves the rule open), one judging by a `Joined` list (its
expression form, for the catalogue, is a `none` aggregate over the list of
an item found) or one whose checks pass where a condition holds
(`unless`).

A `Compare` form expands into an `if` over the operator words, each
branch the test its word names with the target read as a `parameter`. A
rule forked from it states only its own test (`compare`, `oneOf`,
`noneOf`, `isDefined`, `isUndefined`) with its target a literal. The fork
reaches the template's verdicts wherever the value is of the target's kind
or absent (an absence is a missing-information finding, a finding as the
template's); a value of another kind, which the template fails, leaves the
fork open, and blank text is defined to it. A declared tolerance or date
precision has no expression form, so such a rule is not forked
(`ForkError::Inexpressible`).

## Export

A rule bound to a template is still a rule of that capability with its
parameters, so every export profile writes it, or refuses it, exactly as
before the rebuild: IDS refuses `body-extent` and `property-predicate` as
capabilities it has no facet for. A forked rule is an `expression` rule: a profile exports it only
where its `expression_kinds` state every node, and otherwise refuses it
naming the first node it cannot state (IDS: the subtraction of the
rounding allowance). `crates/packages/ids/tests/export.rs` pins both.

## Proving the rebuild

`body-extent` is held to the implementation it replaced in three places:

- **Fixtures and generated inputs.** Every fixture test of
  `crates/engine/rules/tests/body_extent.rs` runs the template and the
  replaced implementation and compares them under `Parity::contract()`,
  each wall's measured extent included; so do the generated walls of
  random size, heading, slack and stated thickness. The replaced
  implementation stays in `body_extent/reference.rs`, compiled only with
  the rules crate's `parity-reference` feature (its own tests turn it on
  through a dev-dependency on itself) and exported as
  `axioval_rules::reference::BodyExtent`. It is no capability of any
  registry: the id resolves to the template.
- **Public models.** The cases `elements` and `unmeshed` list the
  `body-extent` rules as `recorded`: their outcomes on every pinned model,
  as the replaced implementation reported them, are stored beside the case
  and compared with the template's (see [Parity harness](./parity.md)).
  The template reproduces the rule-scoped outcome without geometry (D4,
  D18).
- **The fork.** The forked rule reaches the template's verdicts.

`triangle-count` is held the same way: every fixture of
`crates/engine/rules/tests/triangle_count.rs` and generated meshes of
random count and exactness against
`axioval_rules::reference::TriangleCountLimit` under `Parity::contract()`,
each object's count compared exactly; the `element-triangles` rules of
the cases `elements` and `unmeshed` recorded before the switch (the
rule-scoped outcome without the service, D4); and the fork.

`plan-area` is held so too: every fixture of `plan_area.rs` and
`storey_metrics.rs` runs the template and
`axioval_rules::reference::PlanAreaRange` under `Parity::contract()`
(`Model::holding_contract`, the tests' shared helper), the `areas` table's
values included; generated spaces and storeys of random footprints,
slack, membership and bounds likewise; the `plan-area` rules of the case
`storeys`, recorded before the switch, with their tables; and the fork,
on every fixture whose members are decided.

`property-predicate` is held to its replaced implementation
(`property_predicate/reference.rs`,
`axioval_rules::reference::PropertyPredicate`) the same way: every fixture
of the rules crate that runs it (its own tests and those of dates,
tolerances, quantities, complex and unreadable values) goes through
`common::predicate`, which runs both under `Parity::contract()`;
generated predicates (every operator word, target kind, text option,
tolerance and precision against values of every kind, absent, `null`,
lists, measured intervals and complex properties included) do too; the
`elements` case records `wall-width` and `slab-depth` on every public
model; and its fork reaches its verdicts on values of the target's kind.

`object-count` is held to `object_count/reference.rs`
(`axioval_rules::reference::ObjectCount`) by every fixture of
`tests/object_count.rs` and `tests/resources.rs`, which evaluate
`common::Held`, a capability running the template and the reference on
one context under `Parity::contract()`; by generated projects of sources,
kinds, readable and unreadable ratings and declared disciplines, counted
by kind or rating, per source or across sources, with or without bounds;
and by the `counts` case, whose `object-count` rules were recorded before
the switch. It is never forked.

`related-count` is held to `related_count/reference.rs`
(`axioval_rules::reference::RelatedCount`) the same way: every fixture of
its module in `tests/semantic.rs` and of `tests/relationship_paths.rs`
through `common::Held`; generated rooms and doors, bounded at random,
some doors fire-rated or unreadable, counted along the relationship or in
the whole source, with or without a filter, shared ends and bounds; the
`related-count` rules of the `counts` case, recorded before the switch;
and its fork, which reaches its verdicts on every fixture it forks.

`level-spacing` is held to `level_spacing/reference.rs`
(`axioval_rules::reference::LevelSpacing`) through `common::Held` on
every fixture of `tests/level_spacing.rs` and
`tests/level_spacing_geometry.rs`, the template reading its measured
values as a run does (`Model::evaluate_measured`); by generated buildings
of storeys at random elevations (some unstated or bare numbers), walls
and spaces of random, inexact or unmeasured extents, under random bounds,
consistency, ignored ends, contents and space checks, tables included
(`Model::holding_contract`); and by the `level-spacing` rules of the
`storeys` case, recorded before the switch, tables included. It is never
forked; its measured values (`level_rise`, `prevailing_rise`,
`prevailing_elevation`) still measure through the capability's own
readers in `level_spacing.rs`.

`unique-value` is held to `unique_value/reference.rs`
(`axioval_rules::reference::UniqueValue`) through `common::Held` on every
fixture of `tests/semantic.rs`, `tests/dates.rs` and `tests/tolerance.rs`
that runs it; by generated spaces on two storeys in two sources stating
text, numbers, quantities, nothing, `null` or something unreadable,
under every combination of strictness, scope and tolerance; and by the
`unique-value` rules of the `counts` case, recorded before the switch.
It is never forked.

`consistent-value` is held to `consistent_value/reference.rs`
(`axioval_rules::reference::ConsistentValue`) through `common::Held` on
every fixture of its module in `tests/semantic.rs`, its refusals and
straddling groups worded literally; by generated walls and doors of two
types (some without one) in two sources and on two storeys, stating text,
lengths, areas, numbers, measured intervals, nothing, `null` or something
unreadable, under every combination of strictness, scope, kind and
tolerance; and by the `consistent-value` rules of the `judges` case,
recorded before the switch. It is never forked.

`selector-conformance` is held to `conformance/reference.rs`
(`axioval_rules::reference::SelectorConformance`) through `common::Held`
on every fixture of its module in `tests/semantic.rs`; by generated
spaces stating agreed, unknown, folded, blank, `null`, integer or
unreadable names and long names, against an agreed list, a negation, a
related selector and an expression, with and without a message; and by
the `selector-conformance` rules of the `judges` case, recorded before the
switch. The IDS conformance corpus, whose facets it checks, reports the
same verdicts word for word. It is never forked.

`relative-count` is held to `relative_count/reference.rs`
(`axioval_rules::reference::RelativeCount`) through `common::Held` on
every fixture of `tests/relative_count.rs`, `tests/relative_table.rs` and
its module in `tests/semantic.rs`, every refusal now asserted word for
word; by generated rooms in two sources reaching washbasins, other
fixtures and workplaces (some of undecided kind) along a relationship or
in their whole source, judged by ratios with and without small counts and
by tables, per anchor or per group of codes stated, blank, `null`,
folded or unreadable; and by the `relative-count` rules of the `judges`
case, recorded before the switch. It is never forked.

`property-value` is held to `property_value/reference.rs`
(`axioval_rules::reference::PropertyValueConstraint`) through
`common::Held` on every fixture of `tests/property_value.rs`,
`tests/property_patterns.rs`, `tests/complex_values.rs`,
`tests/dates.rs` and `tests/unreadable_values.rs` that runs it, its
refusals asserted word for word; by generated values of every kind
(text, booleans, integers, decimals, dates, quantities, measured
intervals, `null`, complex values, lists and ranges), declared of one
type, another or none, against generated facets under `optional`,
`si_units` and either quantifier; by the `property-value` rules of the
`judges` case, recorded before the switch; and by the IDS conformance
corpus, whose value facets it checks, reporting the same verdicts word for
word. It is never forked.

`property-requirements` is held to `property_requirements/reference.rs`
(`axioval_rules::reference::PropertyRequirements`) through
`common::Held` on every fixture of `tests/property_requirements.rs`, its
refusals asserted word for word; by generated walls stating text,
numbers, lengths, lists, blanks, `null` or nothing in two sets, some
unreadable, against generated rows of every statement, with and without
grouping by value and a category; by the `property-requirements` rules
of the `judges` case, recorded before the switch; and by the IDS
conformance corpus, reporting the same verdicts word for word. It is
never forked.

`property-comparison` is held to `property_comparison/reference.rs`
(`axioval_rules::reference::PropertyComparison`) through `common::Held`
on every fixture of `tests/property_comparison.rs`,
`tests/property_comparison_scopes.rs`,
`tests/property_comparison_targets.rs`, `tests/dates.rs`,
`tests/tolerance.rs`, `tests/quantity_targets.rs` and
`tests/complex_values.rs` that runs it, its refusals asserted word for
word; by generated rooms of chairs stating values of every kind (some
unreadable), compared each, at least one, by count and by sum, against
constants, properties, text lists and ranges, scaled and under a
tolerance or not; and by the `property-comparison` rules of the `judges`
case, recorded before the switch. It is never forked.

`shelf-capacity` is held so too: every fixture of `shelf_capacity.rs`
and generated spaces of random doors, heights and minimums against
`axioval_rules::reference::ShelfCapacity` under `Parity::contract()`, the
messages asserted literally; the case `shelving` records its rules on
every public model before the switch (the public models' spaces all leave
the linear-quantity service unable to measure, so they hold the
refusals, not the layout); and the fork, an `expression` rule passing the
rule's selectors (`doors=@door_selector`) and reaching the template's
verdicts, compared uncounted (D1).

`area-ratio` is held to `area_ratio/reference.rs`
(`axioval_rules::reference::AreaRatio`) on every fixture of
`plan_area.rs` and `storey_metrics.rs` under `Parity::contract()`, the
`ratios` table's values included; by generated storeys of spaces and
slabs of random footprints, slack, statements and membership under random
bounds, populations, stated sides, empty-numerator findings and
traversals, and generated rooms of windows of random stated light areas,
sizes, type names, light-area rows, tolerances and frame allowances; and
by the `area-ratio` rules of the `storeys` case, recorded before the
switch, tables included. It is never forked (D21, D22).

`plan-coverage` is held to `plan_coverage/reference.rs`
(`axioval_rules::reference::PlanCoverage`, which shares the search) on
every fixture of `plan_area.rs`; by generated spaces and compartments of
random extents, slack, measurement and membership, judged everywhere in
the source, along a relationship or along a path, under random minimums;
by the `space-on-slab` rule of the `storeys` case, recorded before the
switch; and its fork, which carries the selector, the minimum and the
traversal, reaches its verdicts (D23).

`ramp-geometry` is held to the implementation it replaced
(`axioval_rules::reference::RampGeometry`) through `common::Held` on every
ramp fixture of `tests/stair_geometry.rs` (`held`), its messages asserted
literally; by generated ramps of random runs, widths and landings under
random slope limits, tolerances, widths and landing minimums; and by the
`ramp-geometry` rules of the case `stairs`, recorded before the switch
(the public models hold no ramp: they record the walking-surface
service's refusals). It is never forked. A selector whose objects cannot
all be listed leaves each check open once, worded as the binding words it,
where the capability left each end open naming the selection (D24).

`stair-geometry` is held the same way to
`axioval_rules::reference::StairGeometry`: every flight and whole-stair
fixture of `tests/stair_geometry.rs`; generated flights of random risers,
margins, widths and landings under random step, count, rise, tolerance,
width and landing bounds; and the `stair-geometry` rules of the case
`stairs`, recorded before the switch. Both references are compiled only
with `parity-reference` (`stair_geometry/reference.rs`).
`slab-contact` is held to `slab_contact/reference.rs`
(`axioval_rules::reference::SlabContact`, which shares the storey search)
on every fixture of `tests/slab_contact.rs`, the template measuring exactly
the requests the capability sent (a face on a storey left out is never
measured); by generated walls on storeys of random elevations (some
unstated), resting on slabs a selector picks surely, not or undecidably,
with random contact, gaps, minimums, sides and storeys left out; by the
`slab-contact` rules of the `coverage` case, recorded before the switch;
and its fork, which passes a face where a value leaving it unjudged holds
and otherwise reaches the template's verdicts, its findings at the rule's
severity (D25).

`slab-stack-spacing` is held to `slab_stack/reference.rs`
(`axioval_rules::reference::SlabStackSpacing`, which shares the stack
search) through `common::Held` on every fixture of `tests/slab_stack.rs`;
by generated stacks of slabs of random plans, elevations, thickness and
tessellation under random bounds, consistency and tolerances; and by the
`slab-stack` rule of the `storeys` case, recorded before the switch. It is
never forked (D26).

`counterpart-coverage` is held to `counterpart_coverage/reference.rs`
(`axioval_rules::reference::CounterpartCoverage`, which measures through
the same `Subject`) on every fixture of `tests/counterpart_coverage.rs`,
its messages asserted literally; by generated walls, structure and beams of
random plans along either axis, heights and chord deviations, some
counterparts without geometry or of undecided selection, under random
tolerances (some switched off), bands, axis tolerances, elevations and
frames; and by the `counterpart-coverage` rules of the `coverage` case,
recorded before the switch (D27).

`effective-coverage` is held to `effective_coverage/reference.rs`
(`axioval_rules::reference::EffectiveCoverage`, which measures through the
same `Element`) on every fixture of `tests/effective_coverage.rs`, the
template asking the plan-area service exactly what the capability asked;
by generated rooms, devices of random effects (some of undecided
selection, some without geometry), walls, stated areas and capacities
(some missing, `null` or of another kind) under every mode, minimum and
capacity declaration; by the `effective-coverage` rules of the `coverage`
case, recorded before the switch; and its fork, which requires `reaching`
stated and the share at least the minimum and reaches its verdicts. A rule
declaring a capacity is never forked (D27).

`recess-width` is held to `recess_width/reference.rs`
(`axioval_rules::reference::RecessWidth`, which reads and selects rows as
the measured recesses do) through `common::Held` on every fixture of
`tests/recess_width.rs`, its messages asserted literally; by generated
spaces of random recesses (exact and inexact widths and depths, some
spaces unmeasured or measured only approximately) under random rows; and
by the `recess-width` rules of the `spans` case, recorded before the
switch. It is never forked. The capability's undecided row (D3) is kept:
a recess whose depth straddles a row's bound is open, whatever its width.

`light-well` is held to `light_well/reference.rs`
(`axioval_rules::reference::LightWell`) through `common::Held` on every
fixture of `tests/light_well.rs`, its messages asserted literally; by
generated wells of random stacks (exact and inexact extents, gaps and
overlaps, some spaces unmeasured), sections (empty, inexact) and rows
under random tolerances; and by the `light-well` rules of the `spans`
case, recorded before the switch. It is never forked.

`centre-line-distance` is held to `centre_line_distance/reference.rs`
(`axioval_rules::reference::CentreLineDistance`, which reads the walls and
the centre line as the measured sides do) on every fixture of the facade's
`tests/axiolid_centre_line_distance.rs`, measured on real meshes, its
messages and refusals asserted literally; by generated stalls of WCs at
many distances from one or two walls, along every centre line, on one
side or both, under many bounds; and by the `centre-line-distance` rules
of the `spans` case, recorded before the switch. It is never forked
(D28).

`component-visibility` is held to `component_visibility/reference.rs`
(`axioval_rules::reference::ComponentVisibility`, which searches the same
view) through `common::Held` on every fixture of
`tests/component_visibility.rs`, its messages and refusals asserted
literally; by generated scenes of doors at many distances, in view, hidden
by a wall or by a pillar of undecided selection, or undecided, some
answered approximately, under either mode and many minimums; and by the
`component-visibility` rules of the `proximity` case, recorded before the
switch. It is never forked (D29).

`exit-separation` is held to `exit_separation/reference.rs`
(`axioval_rules::reference::ExitSeparation`, which reaches and measures
through the same `Reached` and `Measured`) through `common::Held` on every
fixture of `tests/exit_separation.rs`, its messages and refusals asserted
literally; by generated rooms of up to four doors, tagged, untagged or of
unreadable tag, at exact, tessellated or unmeasured distances, under exact,
interval or missing diagonals, every pair mode, minimum and flag (stated
true or false, unreadable, unstated with a default); and by the
`exit-separation` rules of the `spans` case, recorded before the switch.
It is never forked (D31).

`name-sequence` is held to `name_sequence/reference.rs`
(`axioval_rules::reference::NameSequence`, which reads the members through
the same `members`) through `common::Held` on every fixture of its module
in `tests/semantic.rs`, its messages and refusals asserted literally; by
generated buildings of storeys named by numbers, words, blanks, integers or
nothing, some unreadable, at stated, missing or placed elevations, numbered
from a random start by a random increment, along the relationship or in the
whole source; and by the `name-sequence` rules of the `numbering` case,
recorded before the switch. It is never forked (D32).

`numbering-consistency` is held to `numbering_consistency/reference.rs`
(`axioval_rules::reference::NumberingConsistency`, which reads the numbers
through the same `Collected`) through `common::Held` on every fixture of
its module in `tests/semantic.rs`, its refusals asserted literally; by
generated storeys of spaces named with and without the pattern's prefix,
by too few or too many digits, blanks, integers or unreadable values, some
on no storey and one in another source, checked for prefixes and gaps per
storey, per source and across sources; and by the `numbering-consistency`
rules of the `numbering` case, recorded before the switch. It is never
forked (D33).

`wall-spacing` is held to `wall_spacing/reference.rs`
(`axioval_rules::reference::WallSpacing`, which pairs members and measures
bands through the same `Storey`, `Bands` and `uncovered`) on every fixture
of the facade's `tests/axiolid_bays_and_spacing.rs`, measured on real
meshes, its messages and refusals asserted literally; by generated storeys
of walls at random offsets, turns and lengths, some tessellated, under
random minimums, maximums and areas allowed; and by the `wall-spacing`
rules of the `spans` case, recorded before the switch. It is never forked
(D34).

`parking-bay` is held to `parking_bay/reference.rs`
(`axioval_rules::reference::ParkingBay`, which judges the same
`Bay::steps`) on every fixture of the facade's
`tests/axiolid_bays_and_spacing.rs`, measured on real meshes, its
refusals compared word for word; by generated declarations over both car
parks, size bounds, orientations to the aisle, obstructions in findings
mode and filters by orientation (to the aisle or the neighbours) and by
obstructed ends and sides, some objects tessellated; and by the
`parking-bay` rules of the `spans` case, recorded before the switch. It
is never forked (D35).

`distance` is held to `distance/reference.rs`
(`axioval_rules::reference::Distance`, which judges the same `verdicts`)
on every fixture of `tests/distance.rs`, `tests/distance_door_swing.rs`
and the distance tests of `tests/clash.rs`, and on real meshes in the
facade's `tests/axiolid_certified_distance.rs`; by generated declarations
over every mode, both bounds, plan and space projections, straddling,
tessellated and unreadable counterparts and counterparts scoped to
containers of an undecided kind, some declarations refused; and by the
`distance` rules of the `proximity` case, recorded before the switch. It
is never forked (D36).

`containment` is held to `containment/reference.rs`
(`axioval_rules::reference::Containment`, which judges the same
`assess`) on every fixture of `tests/containment.rs`; by generated
declarations over columns in two adjacent walls (containments sure,
straddling and absent, a column at the junction, unmeasured columns and
walls, face distances measured, straddling and refused, inside and
outside bands, orphans, counts and combining, some declarations refused);
and by the `containment` rules of the `proximity` case, recorded before
the switch. It is never forked (D37).
`coordinate-consistency` is held to `coordinate_consistency/reference.rs`
(`axioval_rules::reference::CoordinateConsistencyCheck`, which shares the
comparison and the choice of the reference) through `common::Held` on
every fixture of `tests/coordinate_consistency.rs`, its refusals asserted
word for word; by generated federations of shifted, turned and one-sided
world frames, true norths, map conversions (targets, offsets, rotations,
scales, units stated, defaulted or unknown) and sites, unreadable systems,
a source holding no object and disciplines declared once, twice or not at
all, under random tolerances and requirements; and by the `coordinates`
rule of the `storeys` case, recorded before the switch. It is never
forked; its measured values `coordinate_shift` and the rest, now values
of a source, still reach its verdicts as expressions over each object.

`external-wall-validation` is held to
`external_wall_validation/reference.rs`
(`axioval_rules::reference::ExternalWallValidation`) through
`common::Held` on every fixture of `tests/external_wall_validation.rs`,
every message asserted word for word, the service asked once per
derivation; by generated walls in two sources declared, derived and of
unknown declaration at random, under one or both derivations in either
order, bounded by decided, undecided or empty selections, with the
service answering or refusing; and by the `external-walls` rule of the
`coverage` case, recorded before the switch. It is never forked (D30).

`horizontal-guard` is held to `horizontal_guard/reference.rs`
(`axioval_rules::reference::HorizontalGuard`, which shares the search and
the filters with `guard_edges`) on every fixture of
`tests/horizontal_guard.rs` (`held`), its refusals asserted word for word;
by generated slabs of random edges, barriers (some on curbs), landings and
climbable objects under random thresholds, curb measurement and role
selections; and by the `horizontal-guard` rules of the `spaces` case,
recorded before the switch (the public models' guard requests are refused,
so they hold the rule's refusal). It is never forked.

`space-boundary-coverage` is held to `space_boundary_coverage/reference.rs`
(`axioval_rules::reference::SpaceBoundaryCoverage`) on every fixture of
`tests/space_boundary_coverage.rs`, its refusals and joined open outcomes
asserted word for word; by generated spaces of random surfaces, uncovered
and overlapping parts (some intervals, some inexact), boundaries on and
off the surface (some without an element), pairs overlapping surely or
possibly, and spaces the service refuses, under every combination of the
three bounds and plane tolerances; by the `space-boundary-coverage` rules
of the `spaces` case, recorded before the switch; and its fork, an
expression requiring no boundary off the surface and every declared
check, which reaches its verdicts (one finding where the template may
report several, as any fork).

`opening-area` is held to `opening_area/reference.rs`
(`axioval_rules::reference::OpeningArea`) on every fixture of
`tests/opening_area.rs` under `Parity::contract()`, its refusals asserted
word for word and side areas stated of every kind (an area, `null`, a
length, a text, an integer, unreadable or absent) on either side; by
generated walls of up to three openings, through the wall or recessed,
stating random side areas, one or none, under random tolerances and
minimum areas; by the `opening-area` rules of the case `openings`,
recorded before the switch; and by its fork on walls stating both areas.

`corridor-end-openings` is held to `corridor_end_openings/reference.rs`
(`axioval_rules::reference::CorridorEndOpenings`) on every fixture of
`tests/corridor_end_openings.rs` under `Parity::contract()`, its undecided
openings and refusals worded literally; by generated corridors of two
ends (each a wall or undecided) and three windows of random gap and
facing, selected by kind or by a kind that may not be read, under random
margins; and by the `corridor-end-openings` rules of the case `openings`,
recorded before the switch. It is never forked.

`space-connection` is held to `space_connection/reference.rs`
(`axioval_rules::reference::SpaceConnection`) on every fixture of
`tests/space_connection.rs` under `Parity::contract()`, its messages
asserted literally (forbidden and missing links and exits, faces of a
door, undecided doors, stated boundaries, refused declarations); by
generated tables of up to three rows over the flat of three rooms (a
`from` by kind or by a use stated, unreadable or absent; random `to`,
requirements, access types, exits and labels, refused ones included) with
doors of declared or undecided type and an unreadable element; and by the
`space-connection` rules of the case `openings`, recorded before the
switch. It is never forked.

`opening-spaces` is held to `opening_spaces/reference.rs`
(`axioval_rules::reference::OpeningSpaces`) on every fixture of
`tests/opening_spaces.rs` under `Parity::contract()`, its messages asserted
literally (stated boundaries, faces of the derived adjacency, a space on no
face, sources without an external wall or with one undeclared, an element
without a host, refused paths); by generated walls declaring their
exposure true, false, `null`, as text or not at all, elements hosted by
them or by an object the host selector cannot decide, related to random
spaces through stated boundaries or the derived adjacency on random faces,
spaces picked by kind or by a use some state unreadably; and by the
`opening-spaces` rules of the case `openings`, recorded before the switch.
It is never forked.

`door-swing` is held to `door_swing_direction/reference.rs`
(`axioval_rules::reference::DoorSwing`) on every fixture of
`tests/door_swing.rs` under `Parity::contract()`, its outcomes worded
literally (selections that cannot decide a space, both directions
together, refused declarations and services); by generated doors of six
leaves opening onto random rooms whose uses are stated, unreadable or
absent, judged by kind or by use in either direction; and by the
`door-swing` rules of the case `openings`, recorded before the switch. It
is never forked.

`empty-host` is held to `empty_host/reference.rs`
(`axioval_rules::reference::EmptyHost`) on every fixture of
`tests/empty_host.rs` under `Parity::contract()`, its refusals asserted
word for word; by generated walls of up to three openings of random size
and place (overlapping, side by side, reaching past the wall, small)
under random tolerances, minimum areas and faces; by the `empty-host`
rules of the case `openings`, recorded before the switch; and by its fork,
the truth itself, on the fixtures.

`opening-zone` is held to `opening_zone/reference.rs`
(`axioval_rules::reference::OpeningZone`) on every fixture of
`tests/opening_zone.rs` under `Parity::contract()`, its messages asserted
literally (beams, webs and flanges, L sections, mitred and notched
outlines, zones, the dimensioning table, supports by path and by contact,
the minimum area, ducts through several beams); by generated beams holding
up to four holes of random place, outline and kind, hosted, unhosted or
with a body the set cannot bound, beside random columns, under random end,
edge and far-edge distances, spacing, zones, dimension rows, minimum areas
and support requirements; and by the `opening-zone` rules of the case
`openings`, recorded before the switch. It is never forked.

`keyed-limit` is held to `keyed_limit/reference.rs`
(`axioval_rules::reference::KeyedLimit`) on every fixture of
`tests/keyed_limit.rs` and `tests/keyed_limit_pairs.rs` under
`Parity::contract()`, its messages asserted literally (every quantity,
path and pair keys, derived groups and classifications, no row, ties,
refused declarations, sill heights above unknown and tessellated floors,
threshold steps with and without ramps); by generated doors keyed by
operation types a row selects, none selects or that cannot be read, with
stated, unreadable and overall widths under random deductions and
bounds, and generated windows beside random spaces, some unmeasured, some
tessellated; and by the `keyed-limit` rules of the case `openings`,
recorded before the switch. It is never forked.

The reference is kept, rather than deleted, because generated inputs need
a live implementation to compare with; recorded outcomes outlive it on the
public models. A rebuild retires its reference once its family's
templates are proven (the rebuild issue decides), deleting the module and
its call sites' entries in `scripts/measurement_ledger.json`, while the
recorded outcomes and the fixtures' literal messages remain.

## Extension points for the rebuilds

The rebuilds of #280–#287 extend the form where a family needs it, each
addition data the runner interprets, never code per capability:

- **Decisions.** `Decision` is an enum: add a variant for a judge a
  family shares (a truth `Requirement` expression over the values, worded
  by which labelled operand failed; consistency; `Unique` is built), with its
  expression form for expand and fork. Decide every comparison through
  `axioval_engine::comparison`, the one implementation the evaluator, the
  selectors, the judges and `Within` share (#287); `Compare` is the
  comparison judge the property judges build on.
- **Grading** (built, `Template::grades`; `Form::grading`). A decision
  reports the deviation from the declared bound, never the widened one, for
  the runtime's severity bands; a capability choosing its findings'
  severities itself states them as bands chosen by conditions over values
  read to grade the finding (#282).
- **Rule parameters and the anchor as arguments** (built, #289). A value
  hands its measurement the rule's parameters or the anchor (`@name`,
  `@anchor`), typed by the registry and bound per object; a provider
  cites what it measured against, and `related` relates it.
- **Scopes and related objects** (members built, `Form::members`; scopes
  built, `Form::scope`). An anchor judged through the members a selector
  parameter picks, read as an aggregate over them, findings relating the
  decided members; a source or the project judged over the objects the
  rule selects there; every source judged by values of its own, or the
  sources holding a selected object and then each object
  (`ScopeSources`, #291).
- **Values of a source or the project, read once per rule** (built,
  #291): a measured value whose subject is a source or the project
  (`MeasuredSubject`), `Form::once`, `Condition::Measured`, and
  `Decision::Joined`, one outcome joining the words of a list's items.
- **Several findings per object** (built: `Judgement`s of
  `Decision::Each` for members, `Form::checks` for an object-level form,
  each its own outcome; divergence D1; and `Decision::Items`, each item of
  a measured list judged on its own).
- **Searches as measured values** (built for stairs and ramps: a search
  answers one three-valued item per searched item, `found` and its
  words).
- **Tables and defaults.** Table parameters reach values through `lookup`;
  fallbacks (a stated value, then a table, then a declared default, each on
  an exact absence only) are a value list read in order, the first that is
  not absent winning, and every default used is worded and cited.
- **Searches.** A search stays Rust: a measured value (or member list)
  registered by a provider beside the capability, which a value reads by
  name. Its results become values like any measurement.
- **Messages.** Add a placeholder format where a family words values
  another way (areas, angles, counts), in `templates.rs`.
- **Services.** Add a `Service` variant for each host service a template
  needs before selection.
- **Report tables** (built, `Form::table`). A template whose capability
  reports a table states its columns as values to tabulate, so the table
  stays a report contract; a column id may name a text.

## Performance

A template must stay within the budget of the code it replaces: at most
1.25 times its run time and 1.5 times its peak heap. How the runner keeps
there, and the benchmark that holds every rebuild to it, are in
[Template performance](./performance.md). `body-extent` reads three
measured values per object (`body_extent` and both `body_position` ends)
where the capability measured once; its provider measures each body's
frame once and its extent once per axis for the whole run, so the three
values, and every rule reading them, share one measurement.

## Rebuilding a capability as a template

1. Pin the contract: the capability's entry in
   `tests/golden/descriptors.json` must not change, and its fixture tests
   assert their messages literally.
2. Make sure every measurement the capability takes is a registered
   measured value (`scripts/measurement_ledger.json` maps each call site),
   including any magnitude its rounding allowance scales with.
3. Write the template as data in `<capability>/template.rs`: descriptor,
   defaults, declaration checks in the capability's order and wording,
   services, texts, forms with values, decision and messages.
4. Move the implementation to `<capability>/reference.rs` behind
   `parity-reference`, export it from `axioval_rules::reference`, and let
   the capability's type run the template (`templates::run`, returning the
   template from `RuleCapability::template`). Rename its ledger entries to
   the reference module.
5. Run every fixture under `Parity::contract()` against the reference,
   measured values included, and the generated inputs likewise; fork each
   fixture's rule and compare outcomes.
6. List the capability's rules in a public parity case as `recorded`,
   record them with `AXIOVAL_PARITY_RECORD=1` **before** switching, then
   switch and compare. Record any divergence with its reason and decision.
7. Bless the catalogue (`AXIOVAL_BLESS=1 cargo test -p axioval-rules --test
   catalogue`); never the descriptors.
8. Add the capability to the template benchmark (`PAIRS` in
   `crates/apps/cli/benches/templates.rs`: its id, its parity case and its
   reference) and run its gate, `python3 scripts/bench.py gate`; the
   rebuild is done only within the budget (see
   [Template performance](./performance.md)).
9. Document the template here and in the capability's section.
