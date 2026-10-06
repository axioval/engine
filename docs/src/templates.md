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
follows `unique-value` as a group decision (#287).

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
| `refusals` | Where a refused declaration and missing services are reported: `rule` (the default, once, before anything is selected, after the name) or `objects` (for each selected object, worded as the check states it, as capabilities that judged their declaration per object reported it). |
| `defaults` | Values optional parameters take when unstated (`tolerance` 0 m). |
| `declaration` | `Check`s over the rule's parameters, in order: `excludes` (where a mode is stated, no string parameter of a list states an option it does not combine with), `arguments` (where a mode is stated, the rule parameters a measured value names, checked as stated by the value's own argument check: a declaration, such as a table of rows, only the measurement knows how to read), `choice` (a string among options), `length` (a non-negative length), `count` (an integer of at least zero, stated where the descriptor requires it), `kind` (a parameter of its descriptor's kind, placed where the capability read it so refusals keep their order), `nonNegative` (numbers of at least zero), `traversal` (a valid `relationship` or `path`, declared only with one of the named parameters, or anywhere where it names none), `exclusive`, `anyOf`, `requires`, `ordered` (numbers, integers or quantities, as the descriptor types them), `disciplines` (a non-empty list of valid disciplines), `path` (a valid relationship path), `tolerance` (the rule's tolerance parameters valid together), `required` (a parameter stated, of its kind), `finite` (each parameter stated as a finite `number`, above or at least a bound where given), `increasing` (both numbers, the first below the second), `atMost` (each number stated at most a value: a share no greater than the whole). The first failing check leaves the rule not evaluated as an invalid declaration, worded as the capability worded it. |
| `services` | The host services the values need (`Service`: `object-frame`, `vertical-extent`, `triangle-count`), and the message leaving the whole rule open without them, before anything is selected. |
| `texts` | Named message parts, optionally conditional: `Condition::Positive` (a parameter above zero), `Condition::Inexact` (a value read from evidence that is not exact, such as a count of a tessellation), `Condition::Equals` (a string parameter or its default is a value), `Condition::Zero` (a value's lower end is zero: nothing surely counted), `Condition::All` (every one of several), `Condition::Not` and `Condition::Cites` (a value's measured reads cite an object: a search found a candidate). Several texts may share a name, each under its own condition: the first that holds is rendered (`plan area` or `facade area` by `measure`). |
| `forms` | The compositions. The first form whose `when` parameters are all stated applies. |

A `Form` holds:

- `values`: `TemplateValue`s, each a named expression, read in order for
  each selected object. A value may `expect` a kind (`length`), and words
  a stated absence (`absent`) and a value of the wrong kind (`mismatch`);
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
  cannot be read leaves the object open once.

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
intervals, as the capabilities computed them. A ratio whose denominator
may be zero has no upper bound (infinity), where the evaluator's
division refuses a divisor holding zero, so a minimum it surely exceeds
still decides; a denominator surely zero leaves the object open with
`zero`. A form with derived values is not forked (its requirement, for the
catalogue, states a ratio as a division and a difference as a
subtraction); one with further populations forks each into an aggregate
along the same path, filtered by its selector. `area-ratio` runs on both;
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
`declares` (one stated, a boolean true) and `falseRequires` (a boolean
stated false needs one of some parameters).

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
constant, with three decimals), `{bound}` (the bound a `Within` failed or straddled, a length:
`at least 0.26 m`), `{bound:plain}` (the bound as declared: `at least 6`),
`{why}`, an anchor's `{undecided}` and `{relation}`, and a value's lower
end to two decimals (`{numerator:least2}`: `0.66`), and a share's upper
end, at most 1, to four decimals (`{share:share}`: `0.4`); a `Compare`
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

A form with a `scope` expands into its decision over the aggregate of the
rule's `selection`, as the catalogue shows it, but is never forked
(`ForkError::Inexpressible`): an `expression` rule judges objects, never
a source or the project as a whole.

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
- **Grading** (built, `Template::grades`). A decision reports the
  deviation from the declared bound, never the widened one; severity bands
  stay the runtime's.
- **Rule parameters and the anchor as arguments** (built, #289). A value
  hands its measurement the rule's parameters or the anchor (`@name`,
  `@anchor`), typed by the registry and bound per object; a provider
  cites what it measured against, and `related` relates it.
- **Scopes and related objects** (members built, `Form::members`; scopes
  built, `Form::scope`). An anchor judged through the members a selector
  parameter picks, read as an aggregate over them, findings relating the
  decided members; a source or the project judged over the objects the
  rule selects there.
- **Several findings per object** (built: `Judgement`s of
  `Decision::Each` for members, `Form::checks` for an object-level form,
  each its own outcome; divergence D1).
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
