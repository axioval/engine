# classification — identifying what a thing *is*

**Status: design. No implementation yet.** Owned by Agent SPEC (PLAN §5).

This module answers one question:

> Given a component in a model, which concept does it belong to?

That sounds trivial. It is the hardest unsolved problem in the checker, and this
document is an honest account of why — written before the code, so the code does
not quietly assume the easy version of the problem.

---

## 1. The worked example: DIN 276 cost groups

DIN 276:2018-12 is the German standard for construction cost grouping. Four
neighbouring cost groups (*Kostengruppen*, KG):

| KG | Name |
|---|---|
| 331 | Tragende Außenwände (load-bearing external walls) |
| 332 | Nichttragende Außenwände (non-load-bearing external walls) |
| 341 | Tragende Innenwände (load-bearing internal walls) |
| 342 | Nichttragende Innenwände (non-load-bearing internal walls) |

Two boolean axes — load-bearing, external — over one element type. This looks
like the easiest classification problem imaginable. It is not.

### 1.1 IFC already gives two legitimate encodings

**Explicit**, via the classification mechanism:

```
IfcClassification("DIN 276")
  └── IfcClassificationReference("331")
        └── IfcRelAssociatesClassification → IfcWall
```

**Implicit**, via the standard property sets:

```
IfcWall
  + Pset_WallCommon.LoadBearing = TRUE
  + Pset_WallCommon.IsExternal  = TRUE
```

Both are *correct* IFC. A rule written against one silently returns nothing on a
model authored the other way. Neither is more canonical than the other, so a
classification language must accept both as evidence for the same concept.

This is the shape of the whole problem: **many faces, one concept.**

### 1.2 Then reality arrives

In real project models we routinely see:

- `IfcBuildingElementProxy` instead of `IfcWall`
- German property names (`Tragend`, `Aussen`) in a custom Pset
- `LoadBearing` as the string `"Ja"`, or `1`, or absent
- the cost group written into `Name` as `"331_AW_tragend"`
- nothing at all — geometry and a layer name

A human inspector classifies all of these correctly in seconds. Our filter table
needs a new row for each, in every rule that touches walls. That is the pain
this module exists to remove.

### 1.3 The standard itself is not crisp

Read KG 331's actual definition:

> "Außenwände und flächige Konstruktionen, die für die Standfestigkeit des
> Bauwerks erforderlich sind, **einschließlich horizontaler Abdichtungen sowie
> Schlitzen und Durchführungen**."

The cost group is not "the wall". It is the wall **plus** attached
sub-components (horizontal damp-proofing) **plus** *negative* features (chases
and penetrations — voids, not solids). One conceptual unit spans a host element,
its attached parts, and its voids.

And two lines above, the standard says:

> "Die KG 331 und die KG 332 können ggf. als KG 331 zusammengefasst werden."

*The distinction we are trying to detect is optional in the source standard.*

### 1.4 The standard ships its own tie-break

DIN 276 §5.2:

> "Die Kosten sind möglichst getrennt und eindeutig den einzelnen Kostengruppen
> zuzuordnen. Bestehen **mehrere Zuordnungsmöglichkeiten** und ist eine
> Aufteilung nicht möglich, sind die Kosten entsprechend der **überwiegenden
> Verursachung** zuzuordnen."

The standard *anticipates* multiple valid assignments and resolves them by
**predominant causation**. So ambiguity is not a defect in our model of the
domain — it is a documented feature of the domain, and a faithful IR must be
able to express "several matched; here is why this one won."

This single sentence justifies most of the design below.

---

## 2. Why the existing tools do not cover it

### the provider filters

`ClassAndPropertyFilter` = class + property + operator + value, composed into
And/Or/Not trees. Every project exception becomes another row, duplicated into
every rule that needs it. No name, no provenance, no reuse.

### the provider classifications

Better: a **named** artifact with ordered rows, a `MatchMethod`
(`FirstMatch`/`BestMatch`/`AllMatching`), provenance (`source`, `edition`), and
formula rows. This is the right shape — the exception is absorbed once and rules
stay canonical. But it is a flat table over properties: it cannot express
geometry, topology, or confidence.

### IDS

`applicability` + `requirements` is a clean split, but deliberately binary and
narrow: facets over class, property, material, attribute. No "probably", no
geometric evidence, no aggregation over attached parts. IDS answers *"does this
element comply?"*, not *"what is this element?"* — those are different
questions, and conflating them is why IDS feels too narrow here.

### SHACL / ontologies

Expressive enough in principle, and the closed/open-world distinction is
genuinely useful. Two practical problems: it assumes a clean RDF graph we would
have to build first, and it is still fundamentally boolean. `sh:minCount` cannot
say "this looks 80% like a wall."

---

## 3. The dinosaur problem

The user's framing, kept because it is the clearest statement of the core
question:

> From a limited set of bones — no DNA — a palaeontologist identifies the
> species. They do not need the full skeleton. **What is the least information
> needed for identification? And when two candidates are close, do we just work
> with probabilities?**

Restated for our domain: classification is **evidence accumulation toward a
hypothesis**, not predicate satisfaction. Different evidence has different
diagnostic power:

- a femur may be near-decisive
- a rib is weak evidence, consistent with many species
- one bone can *rule out* a family entirely

Mapped onto walls:

| Evidence | Power |
|---|---|
| `IfcClassificationReference = "331"` | near-decisive |
| `IfcWall` + `LoadBearing` + `IsExternal` | strong |
| `IfcWall` + touches building envelope | moderate |
| thin vertical extruded slab, no properties | weak but real |
| horizontal orientation | **falsifying** — rules out wall |

That last row matters as much as the positive ones. Cheap negative evidence
prunes the hypothesis space fastest, exactly as in the dinosaur case.

---

## 4. Geometry and vision as evidence

The user's second insight, and the one that breaks the current ceiling:

> A trained human can see what a wall is from visuals alone, with no properties
> at all. Treat visuals as just another information source.

Two levels, and they are different in kind.

### 4.1 Geometric evidence (cheap, deterministic, available today)

A wall and a slab can be geometrically identical shapes — the difference is
**orientation relative to a known up-axis**. A wall stands up; a slab lies down.
That is computable right now with `geometry`:

- principal axis vs. up-axis
- aspect ratios (thin in one dimension, extended in two)
- adjacency: bounded by two storeys? touching the envelope?
- topology: hosts openings? is hosted *by* something?

This is not machine learning. It is `query/measure.rs` plus a threshold, and it
should be the first non-property evidence source we add. Much of what people
reach for an LLM for is actually this.

### 4.2 Visual evidence (expensive, probabilistic, genuinely new)

Render the isolated element headlessly, send the image to a vision model, get
back "likely a wall". Viable today, and the right fallback when a model has
generic classes and no useful properties.

Three constraints if we do it:

1. **It is evidence, never an oracle.** It enters the same accumulation
   framework with a weight, and it must be capable of being outvoted.
2. **It must be cached and reproducible.** A classification that changes between
   runs is not auditable, and auditability is the product.
3. **Provenance is mandatory.** "Classified as 331 because an LLM said so"
   must be visible in the result, and distinguishable from
   "because `IfcClassificationReference = 331`".

Note the orientation example proves a subtler point: the *image alone* is
ambiguous between wall and slab. Vision needs the geometric context to
disambiguate. So vision is not a replacement for the other evidence types — it
composes with them.

---

## 5. Design direction

Nothing here is committed. This is the shape the evidence points at.

### 5.0 🚨 The central split: identity is not assignment

**This is the most important decision in the module, and it was nearly missed.**

There are two different questions, and the earlier drafts of this document
conflated them:

| | Question | Depends on | Stable? |
|---|---|---|---|
| **Identity** | *What is this thing?* | the model only | yes — a load-bearing external wall is one regardless of standard |
| **Assignment** | *Which bucket does it go in?* | identity **+** a standard **+** project policy | no — differs per standard, per project, per phase |

Selection answers **identity**. DIN 276, IDS, OmniClass, Uniclass, ORCA AVA are
all **consumers** of identity.

The consequence is the point: **we never map DIN 276 ↔ some other standard.**
Every standard binds to the same intermediate identity representation, so N
standards need N bindings, not N² mappings. Adding Uniclass is a new binding
against unchanged identities. And the hard, expensive, evidence-accumulating
work — is this a wall? is it load-bearing? is it external? — is done **once**,
not re-derived inside every standard's ruleset.

```
        model (many faces)
              │
              ▼
   ┌──────────────────────┐
   │  IDENTITY  (this module)   what IS it?
   │  wall · load-bearing · external · attached-to · voids
   └──────────┬───────────┘
              │            one representation, many consumers
   ┌──────────┼───────────┬──────────────┐
   ▼          ▼           ▼              ▼
 DIN 276    IDS      quantity TO      rule scope
 KG 331   compliance   takeoff       applicability
```

DIN 276's own text supports the split. KG 331 is defined by *properties of the
thing* — "Außenwände … die für die Standfestigkeit des Bauwerks erforderlich
sind" — which is exactly load-bearing + external + wall. The standard is
describing an identity predicate, then attaching a number to it.

### 5.0.1 Where the split is not clean (and why that is fine)

Three honest caveats, all discovered in the standard text:

**Not every cost group is an element.** KG 391 Baustelleneinrichtung, 392
Gerüste, 394 Abbruchmaßnahmen, 396 Materialentsorgung are *activities* —
scaffolding, demolition, disposal. They have no model element to identify. So
the identity layer serves the element-shaped cost groups and simply has nothing
to say about the activity-shaped ones. That is a clean "not applicable", not a
gap in the design.

**Assignment carries project policy that identity must not absorb.** §5.2 allows
further subdivision "nach herstellungsmäßigen Gesichtspunkten (z. B. im Hinblick
auf Vergabe und Ausführung)" — procurement-driven grouping. And KG 331/332 "können
ggf. als KG 331 zusammengefasst werden". Whether to collapse them is a *project
decision*, not a fact about a wall. Identity must stay stable while that varies;
the binding layer holds the policy.

**"Überwiegende Verursachung" is an assignment rule, not an identity rule.**
This corrects §5.3 below. Predominant causation resolves *which cost group pays*
when several apply — it is a property of the DIN binding, not of the element.
Identity has its own, separate ambiguity ("is this a wall or a slab?"), resolved
by evidence weight. **Two distinct ambiguity mechanisms at two layers:**

- identity ambiguity → *what is it?* → evidence accumulation
- assignment ambiguity → *whose cost is it?* → überwiegende Verursachung

Collapsing them into one confidence number would be a modelling error. As the
user put it: predominant causation still has to know the elements first.

### 5.1 Evidence, not predicates

```
Evidence  = a single observation with a source and a weight
Hypothesis = a candidate concept + accumulated evidence + a score
Scheme    = named, versioned, ordered set of concepts + how to resolve conflicts
```

A `Predicate` (the existing filter leaf) becomes **one kind of evidence** rather
than the whole language. That keeps the cheap, deterministic, fully-explainable
path primary, and admits the rest without special-casing.

### 5.2 Keep the boolean core exactly as it is

Most real classification is a boolean expression, it lowers cleanly to `.filter` and
`.classification`, and it is auditable. **Do not replace it.** Layer the
probabilistic machinery *above* it, so a scheme with only boolean evidence
behaves precisely as today and round-trips byte-identically.

If the fuzzy layer is not opt-in, we will have made the common case worse to
serve the rare one.

### 5.3 Confidence is a first-class output, with a resolution policy

Not a float bolted on afterwards. The result of *identity* resolution should be:

```
Resolved   { concept, evidence[] }                      // unambiguous
Ambiguous  { candidates[], why_chosen, alternatives[] } // wall or slab?
Unknown    { closest[], missing_evidence[] }            // say what WOULD decide it
```

`Unknown` returning *what evidence is missing* is what makes the tool useful
rather than merely correct: it tells a modeller what to fix.

> ⚠️ Per §5.0.1, do not use this to model DIN's *überwiegende Verursachung*.
> That is an **assignment** ambiguity living in the binding layer, and it
> consumes an already-resolved identity. Two mechanisms, two layers.

### 5.3.1 Binding: how a standard consumes identity

A binding is deliberately thin — the expensive work already happened:

```
Binding {
  standard: "DIN 276:2018-12",
  rules: [ Concept(wall & load_bearing & external) → "331",
           Concept(wall & !load_bearing & external) → "332", … ],
  policy: { collapse: ["331+332 → 331"],       // §5.2, project decision
            on_multiple: PredominantCausation }
}
```

Note what is *not* here: no property names, no IFC classes, no vendor quirks, no
evidence weights. Those all live in identity. A binding maps resolved concepts
to codes and carries the project policy the standard explicitly permits.

Test of the design: adding OmniClass should require **only** a new binding file,
with zero changes to identity and zero reference to DIN.

### 5.4 A concept is not always one element

KG 331 = wall **+** damp-proofing **+** chases **+** penetrations. The unit of
classification must be able to be a **set with roles** (host, attached part,
void), not a single component. This is the requirement most likely to be
forgotten and most expensive to retrofit — every signature should carry it from
day one.

### 5.5 Provenance is the product

For a cost-group assignment to be defensible in a real project it must state
*why*. Every resolution carries its evidence chain: which properties, which
geometry, which standard clause, which model version. This is also what makes a
fuzzy result acceptable to an auditor — not the number, but the trail.

---

## 6. Open questions

Genuinely unresolved. Do not let an implementation quietly answer them by
accident.

1. **Where do weights come from?** Hand-tuned is unprincipled and will rot.
   Learned needs labelled data we do not have. Possibly: derive from
   corpus-observed discriminative power, like TF-IDF.
2. **How do we validate?** Ground truth is human judgement. The nearest thing to
   an oracle we have is a project's own accepted classification — worth mining.
3. **Is the scheme itself falsifiable?** Two schemes will disagree. Is that a
   scheme bug or a legitimate difference of interpretation?
4. **Where does the cutoff sit?** At what confidence does `Ambiguous` become
   `Resolved`? Almost certainly per-scheme and per-use — a cost estimate and a
   fire-safety check should not share a threshold.
5. **Does it survive lowering?** A fuzzy scheme cannot round-trip to
   `.classification`. Fail loudly, or emit the resolved boolean projection and
   record the loss? (Current lean: fail loudly, per PLAN R12.)

---

## 7. What NOT to do

- **Do not start with the LLM.** It is the most expensive, least reproducible
  evidence source. Property + geometry evidence covers most real cases; adding
  vision first would hide how much the cheap path already achieves.
- **Do not make everything probabilistic.** A model with correct
  `IfcClassificationReference` values must classify deterministically and
  instantly.
- **Do not invent an ontology format.** IFC, IDS and SHACL exist. Lower to them
  where possible; extend only where they provably cannot express the domain.
- **Do not let vendor concepts in.** No `SSpace`, no the provider formula dialect.
  Those belong at the `codec` lowering boundary.
- **Do not conflate applicability with identity.** "Is this a wall?" and "does
  this wall comply?" are different questions. IDS blurs them; we should not.

---

## 8. References

- `data/DIN 276_2018-12-00_DE_2873248.pdf` — the source standard. §5.2 for the
  ambiguity rule, the KG 330/340 tables for the worked example.
- `../../AGENTS.md` — why `classification` lives in `spec` rather than its own crate.
- `../../../codec/src/formats/classification/` — the `.classification` target.
- `../../../rules/COVERAGE.md` — why the rule side needs this: most rule
  templates are format-bound today.
- poing `extensions/apps/orcaavaLrp/` — existing DIN 276 cost-group handling
  worth mining for real-world encodings.
