//! Labels and help of every built-in capability and its parameters, in
//! English and German, for the authoring catalogue.

use axioval_engine::catalogue::ParameterText;
use axioval_ir::measured::{LocalizedText, en_de};

/// One capability's texts.
pub(crate) struct CapabilityText {
    pub(crate) id: &'static str,
    /// A short name for editors (a few words, no trailing period).
    pub(crate) label: [LocalizedText; 2],
    /// What it checks and what leaves it not evaluated, one to three sentences.
    pub(crate) help: [LocalizedText; 2],
    /// Help for each parameter, by parameter name, in the order `parameters()` returns them.
    pub(crate) parameters: &'static [ParameterText],
}

/// One parameter's help.
macro_rules! param {
    ($name:expr, $en:expr, $de:expr $(,)?) => {
        ParameterText {
            name: $name,
            help: &en_de($en, $de),
        }
    };
}

// Relationship traversal, shared by every relationship-scoped capability.

const RELATIONSHIP: ParameterText = param!(
    "relationship",
    "One relationship to walk, such as IfcRelAggregates or a derived relationship; declare it or `path`, not both.",
    "Eine zu durchlaufende Beziehung, etwa IfcRelAggregates oder eine abgeleitete Beziehung; entweder sie oder `path` angeben, nicht beides.",
);
const DIRECTION: ParameterText = param!(
    "direction",
    "Direction of `relationship`: `forward` (default), `backward` or `either`.",
    "Richtung von `relationship`: `forward` (Standard), `backward` oder `either`.",
);
const FOLLOW_CHAIN: ParameterText = param!(
    "follow_chain",
    "Whether `relationship` is followed repeatedly along its chain; false by default.",
    "Ob `relationship` wiederholt entlang ihrer Kette verfolgt wird; standardmäßig false.",
);
const PATH: ParameterText = param!(
    "path",
    "Relationship steps walked in order (`Relationship:direction`, a trailing `+` repeats a step), instead of `relationship`.",
    "Nacheinander durchlaufene Beziehungsschritte (`Beziehung:Richtung`, ein angehängtes `+` wiederholt einen Schritt), anstelle von `relationship`.",
);
const SKIP_ABSENT_RELATIONSHIP_ENDS: ParameterText = param!(
    "skip_absent_relationship_ends",
    "Skip relationship ends the source cannot resolve instead of leaving the object not evaluated; false by default.",
    "Beziehungsenden, die die Quelle nicht auflösen kann, überspringen, statt das Objekt unbewertet zu lassen; standardmäßig false.",
);

// Numeric tolerance of the comparison capabilities.

const TOLERANCE: ParameterText = param!(
    "tolerance",
    "Absolute tolerance within which two numbers count as equal; quantities in their SI unit.",
    "Absolute Toleranz, innerhalb derer zwei Zahlen als gleich gelten; Größen in ihrer SI-Einheit.",
);
const RELATIVE_TOLERANCE: ParameterText = param!(
    "relative_tolerance",
    "Relative tolerance, a fraction from 0 to below 1 of the larger value.",
    "Relative Toleranz als Anteil von 0 bis unter 1 des größeren Werts.",
);
const DECIMALS: ParameterText = param!(
    "decimals",
    "Round both values to this many decimal places (0 to 15) before comparing; not together with a tolerance.",
    "Beide Werte vor dem Vergleich auf so viele Nachkommastellen (0 bis 15) runden; nicht zusammen mit einer Toleranz.",
);

// Direct access between spaces through doors and openings.

const ACCESS_PATH: ParameterText = param!(
    "access_path",
    "Relationship steps from a door or opening to the spaces it connects, such as axioval:derived.adjacent-space.",
    "Beziehungsschritte von einer Tür oder Öffnung zu den Räumen, die sie verbindet, etwa axioval:derived.adjacent-space.",
);
const DOOR_SELECTOR: ParameterText = param!(
    "door_selector",
    "The doors that connect spaces.",
    "Die Türen, die Räume miteinander verbinden.",
);
const OPENING_SELECTOR: ParameterText = param!(
    "opening_selector",
    "The openings without a door that connect spaces.",
    "Die Öffnungen ohne Tür, die Räume miteinander verbinden.",
);
const SPACE_SELECTOR: ParameterText = param!(
    "space_selector",
    "The objects a door or opening may reach as spaces; every object by default.",
    "Die Objekte, die eine Tür oder Öffnung als Räume erreichen darf; standardmäßig jedes Objekt.",
);

// Vertical connectors a walk may climb.

const STAIR_SELECTOR: ParameterText = param!(
    "stair_selector",
    "The stairs a walk may climb to reach other storeys; without any connector, walks stay on their level.",
    "Die Treppen, über die ein Weg andere Geschosse erreichen darf; ohne Verbindungselemente bleiben Wege auf ihrer Ebene.",
);
const RAMP_SELECTOR: ParameterText = param!(
    "ramp_selector",
    "The ramps a walk may climb to reach other storeys.",
    "Die Rampen, über die ein Weg andere Geschosse erreichen darf.",
);
const LIFT_SELECTOR: ParameterText = param!(
    "lift_selector",
    "The lifts a walk may ride to reach other storeys.",
    "Die Aufzüge, mit denen ein Weg andere Geschosse erreichen darf.",
);
const STAIR_LENGTH: ParameterText = param!(
    "stair_length",
    "How a climb counts: `slope` (default, along its slope) or `horizontal-plus-vertical`.",
    "Wie ein Höhenwechsel zählt: `slope` (Standard, entlang der Neigung) oder `horizontal-plus-vertical`.",
);
const VERTICAL_FACTOR: ParameterText = param!(
    "vertical_factor",
    "What one metre of rise counts as with `horizontal-plus-vertical`; at least 0, 1 by default.",
    "Wie viel ein Meter Höhenunterschied bei `horizontal-plus-vertical` zählt; mindestens 0, standardmäßig 1.",
);

// A door's clear width and height, read as `keyed-limit` reads them.

const CLEAR_WIDTH_FROM_LEAVES: ParameterText = param!(
    "clear_width_from_leaves",
    "`passage` or `widest-leaf`: derive the door's clear width from its lining and leaves where none is stated.",
    "`passage` oder `widest-leaf`: die lichte Breite der Tür aus Zarge und Türblättern ableiten, wo keine angegeben ist.",
);
const OVERALL_WIDTH: ParameterText = param!(
    "overall_width",
    "The door's overall width, a length, from which `width_deduction` is subtracted.",
    "Die Gesamtbreite der Tür als Länge, von der `width_deduction` abgezogen wird.",
);
const WIDTH_DEDUCTION: ParameterText = param!(
    "width_deduction",
    "The rule author's approximate deduction for frame, lining and leaf from the overall width, a length of at least zero.",
    "Pauschaler Abzug des Regelautors für Rahmen, Zarge und Türblatt von der Gesamtbreite, eine Länge von mindestens null.",
);
const OVERALL_HEIGHT: ParameterText = param!(
    "overall_height",
    "The door's overall height, a length, from which the lining and threshold are deducted.",
    "Die Gesamthöhe der Tür als Länge, von der Zarge und Schwelle abgezogen werden.",
);
const LINING_THICKNESS: ParameterText = param!(
    "lining_thickness",
    "The head lining thickness the door states, deducted from its overall height.",
    "Die von der Tür angegebene Zargendicke am Sturz, die von der Gesamthöhe abgezogen wird.",
);
const THRESHOLD_THICKNESS: ParameterText = param!(
    "threshold_thickness",
    "The threshold thickness the door states, deducted from its overall height.",
    "Die von der Tür angegebene Schwellendicke, die von der Gesamthöhe abgezogen wird.",
);

// Passing spaces along a route.

const PASSING_WIDTH_METRES: ParameterText = param!(
    "passing_width_metres",
    "Width in metres of a passing space across the route.",
    "Breite einer Begegnungsfläche quer zum Weg, in Metern.",
);
const PASSING_LENGTH_METRES: ParameterText = param!(
    "passing_length_metres",
    "Length in metres of a passing space along the route.",
    "Länge einer Begegnungsfläche entlang des Wegs, in Metern.",
);
const PASSING_SPACING_METRES: ParameterText = param!(
    "passing_spacing_metres",
    "Longest stretch of route in metres without a passing space; declared with its width and length.",
    "Längster Wegabschnitt ohne Begegnungsfläche, in Metern; zusammen mit deren Breite und Länge anzugeben.",
);
const PASSING_REACH_METRES: ParameterText = param!(
    "passing_reach_metres",
    "How far in metres across the route a passing space's centre may lie; half its width by default.",
    "Wie weit quer zum Weg der Mittelpunkt einer Begegnungsfläche liegen darf, in Metern; standardmäßig ihre halbe Breite.",
);

const SUBTRACT_DOOR_SWINGS: ParameterText = param!(
    "subtract_door_swings",
    "The doors whose leaves' swing also counts as an obstacle.",
    "Die Türen, deren Aufschlagbereich der Türblätter ebenfalls als Hindernis zählt.",
);

// Free floor shapes.

const FREE_FLOOR_OBSTACLES: ParameterText = param!(
    "obstacles",
    "What blocks the floor; every other object by default.",
    "Was die Bodenfläche blockiert; standardmäßig jedes andere Objekt.",
);
const FREE_FLOOR_BAND_FROM: ParameterText = param!(
    "band_from_metres",
    "Bottom in metres above the floor of the band in which obstacles count; the floor by default.",
    "Unterkante des Bands über dem Boden in Metern, in dem Hindernisse zählen; standardmäßig der Boden.",
);
const FREE_FLOOR_BAND_TO: ParameterText = param!(
    "band_to_metres",
    "Top in metres above the floor of that band; the shape's height by default.",
    "Oberkante dieses Bands über dem Boden in Metern; standardmäßig die Höhe der Form.",
);
const FREE_FLOOR_MERGE_PATH: ParameterText = param!(
    "merge_path",
    "Relationship path to the spaces searched together with the selected one as one floor.",
    "Beziehungspfad zu den Räumen, die zusammen mit dem gewählten als eine Bodenfläche durchsucht werden.",
);
const FREE_FLOOR_ENTRANCE_PATH_WIDTH: ParameterText = param!(
    "entrance_path_width",
    "Width in metres of a path from the space's entrances that must reach the shape; with `access_path`.",
    "Breite eines Wegs von den Zugängen des Raums in Metern, der die Form erreichen muss; zusammen mit `access_path`.",
);
const FREE_FLOOR_ENTRANCE_TOLERANCE: ParameterText = param!(
    "entrance_tolerance_metres",
    "How much closer than half the path width plus this, in metres, the free area must come to an entrance; 0.05 by default.",
    "Wie nah die freie Fläche einem Zugang kommen muss: halbe Wegbreite plus dieser Wert in Metern; standardmäßig 0,05.",
);

// Clash and clash matrix.

const CLASH_COUNTERPARTS: ParameterText = param!(
    "counterparts",
    "The objects the selected objects are checked against.",
    "Die Objekte, gegen die die gewählten Objekte geprüft werden.",
);
const EXCLUDE_PATHS: ParameterText = param!(
    "exclude_paths",
    "Relationship paths; pairs reaching a shared target through one of them are skipped.",
    "Beziehungspfade; Paare, die über einen davon ein gemeinsames Ziel erreichen, werden übersprungen.",
);
const EXCLUDE_TARGET_PROPERTY: ParameterText = param!(
    "exclude_target_property",
    "Reached targets stating the same value of this property also count as shared, such as one system split across models.",
    "Erreichte Ziele mit gleichem Wert dieser Eigenschaft gelten ebenfalls als gemeinsam, etwa ein auf mehrere Modelle verteiltes System.",
);
const EXCLUDE_SAME_LAYER: ParameterText = param!(
    "exclude_same_layer",
    "Skip pairs of one model sharing a presentation layer; false by default.",
    "Paare eines Modells auf einem gemeinsamen Darstellungslayer überspringen; standardmäßig false.",
);
const GROUP_BY: ParameterText = param!(
    "group_by",
    "`subject`, `type_pair` or `similar`: group reported pairs into one finding each; every pair alone without it.",
    "`subject`, `type_pair` oder `similar`: gemeldete Paare zu je einem Befund zusammenfassen; ohne Angabe jedes Paar einzeln.",
);
const PER_STOREY: ParameterText = param!(
    "per_storey",
    "Also keep groups apart by storey; needs `storey_path`.",
    "Gruppen zusätzlich nach Geschoss trennen; erfordert `storey_path`.",
);
const CLASH_STOREY_PATH: ParameterText = param!(
    "storey_path",
    "Relationship path from an object to its storey, for `per_storey`.",
    "Beziehungspfad von einem Objekt zu seinem Geschoss, für `per_storey`.",
);
const GROUP_PROPERTY: ParameterText = param!(
    "group_property",
    "With `similar`, grouped pairs must also agree on this property's value on both sides.",
    "Bei `similar` müssen zusammengefasste Paare zusätzlich auf beiden Seiten im Wert dieser Eigenschaft übereinstimmen.",
);
const GROUP_TOLERANCE_METRES: ParameterText = param!(
    "group_tolerance_metres",
    "Rounding step in metres of the intersection extents `similar` compares.",
    "Rundungsschritt in Metern der Durchdringungsabmessungen, die `similar` vergleicht.",
);
const SEVERITY_BY_CLASS: ParameterText = param!(
    "severity_by_class",
    "Rows of a clash class (duplicate, containment, intersection, clearance) and the severity its findings take.",
    "Zeilen aus einer Kollisionsklasse (Duplikat, Enthaltensein, Durchdringung, Abstand) und dem Schweregrad ihrer Befunde.",
);
const GRADE_BY: ParameterText = param!(
    "grade_by",
    "`smallest_extent` or `volume`: what intersections are graded by; required with `severity_grades`.",
    "`smallest_extent` oder `volume`: wonach Durchdringungen abgestuft werden; erforderlich mit `severity_grades`.",
);
const SEVERITY_GRADES: ParameterText = param!(
    "severity_grades",
    "Rows of a threshold (metres, or cubic metres for `volume`) and the severity of intersections above it.",
    "Zeilen aus einem Schwellenwert (Meter, bei `volume` Kubikmeter) und dem Schweregrad der Durchdringungen darüber.",
);
const DUPLICATE_QUANTITIES: ParameterText = param!(
    "duplicate_quantities",
    "Rows of a property and optional property set: the quantities a duplicate's copies are compared by.",
    "Zeilen aus Eigenschaft und optionalem Eigenschaftssatz: die Mengen, nach denen die Kopien eines Duplikats verglichen werden.",
);
const TOLERANCE_CASES: ParameterText = param!(
    "tolerance_cases",
    "Rows excusing intersections measured along the elements' own axes: a case, two element filters and a tolerance in metres.",
    "Zeilen, die entlang der bauteileigenen Achsen gemessene Durchdringungen tolerieren: ein Fall, zwei Bauteilfilter und eine Toleranz in Metern.",
);
const PENETRATION_TOLERANCE_METRES: ParameterText = param!(
    "penetration_tolerance_metres",
    "Penetration depth in metres accepted at joints.",
    "An Anschlüssen zulässige Durchdringungstiefe in Metern.",
);
const CLEARANCE_METRES: ParameterText = param!(
    "clearance_metres",
    "Minimum separation in metres; pairs closer than this are clearance clashes.",
    "Mindestabstand in Metern; Paare, die näher liegen, sind Abstandskollisionen.",
);
const DUPLICATE_TOLERANCE_METRES: ParameterText = param!(
    "duplicate_tolerance_metres",
    "Surfaces at most this far apart, in metres, are duplicates; 0 by default.",
    "Oberflächen, die höchstens so weit auseinanderliegen (Meter), sind Duplikate; standardmäßig 0.",
);
const HORIZONTAL_TOLERANCE_METRES: ParameterText = param!(
    "horizontal_tolerance_metres",
    "An intersection must reach further than this, in metres, along both plan axes; 0 by default.",
    "Eine Durchdringung muss entlang beider Grundrissachsen weiter reichen als dieser Wert in Metern; standardmäßig 0.",
);
const VERTICAL_TOLERANCE_METRES: ParameterText = param!(
    "vertical_tolerance_metres",
    "An intersection must reach further than this in height, in metres; 0 by default.",
    "Eine Durchdringung muss in der Höhe weiter reichen als dieser Wert in Metern; standardmäßig 0.",
);
const VOLUME_TOLERANCE_CUBIC_METRES: ParameterText = param!(
    "volume_tolerance_cubic_metres",
    "An intersection must share more volume than this, in cubic metres; 0 by default.",
    "Eine Durchdringung muss mehr Volumen teilen als dieser Wert in Kubikmetern; standardmäßig 0.",
);
const REPORT_DUPLICATES: ParameterText = param!(
    "report_duplicates",
    "Report duplicates; true by default.",
    "Duplikate melden; standardmäßig true.",
);
const REPORT_CONTAINMENT: ParameterText = param!(
    "report_containment",
    "Report bodies lying inside others; true by default.",
    "Körper melden, die in anderen liegen; standardmäßig true.",
);
const REPORT_INTERSECTIONS: ParameterText = param!(
    "report_intersections",
    "Report intersections; true by default.",
    "Durchdringungen melden; standardmäßig true.",
);

// Stair and ramp measurements shared by both.

const MINIMUM_HEADROOM: ParameterText = param!(
    "minimum_headroom",
    "Least vertical clearance above the walking surface; declared with `headroom_obstacles`.",
    "Geringste lichte Durchgangshöhe über der Lauffläche; zusammen mit `headroom_obstacles`.",
);
const HEADROOM_OBSTACLES: ParameterText = param!(
    "headroom_obstacles",
    "Objects that may stand above the walking surface, such as slabs, beams and ducts.",
    "Objekte, die über der Lauffläche liegen können, etwa Decken, Unterzüge und Kanäle.",
);
const WIDTH_MINIMUM: ParameterText = param!(
    "width_minimum",
    "Least width of each flight or run across its walking direction.",
    "Geringste Breite jedes Treppen- oder Rampenlaufs quer zur Laufrichtung.",
);
const WIDTH_MAXIMUM: ParameterText = param!(
    "width_maximum",
    "Greatest width of each flight or run across its walking direction.",
    "Größte Breite jedes Treppen- oder Rampenlaufs quer zur Laufrichtung.",
);
const LANDING_OBJECTS: ParameterText = param!(
    "landing_objects",
    "Objects that may carry a landing, such as landing slabs or floors.",
    "Objekte, die ein Podest bilden können, etwa Podestplatten oder Decken.",
);
const LANDING_DEPTH_MINIMUM: ParameterText = param!(
    "landing_depth_minimum",
    "Least landing depth along the walking direction at each end.",
    "Geringste Podesttiefe in Laufrichtung an jedem Ende.",
);
const LANDING_WIDTH_MINIMUM: ParameterText = param!(
    "landing_width_minimum",
    "Least landing width across the walking direction at each end.",
    "Geringste Podestbreite quer zur Laufrichtung an jedem Ende.",
);
const LANDING_AT_LEAST_WALKING_WIDTH: ParameterText = param!(
    "landing_at_least_walking_width",
    "Each landing must also be at least as deep and as wide as the flight or run.",
    "Jedes Podest muss zudem mindestens so tief und so breit sein wie der Lauf.",
);
const LANDINGS_REQUIRED: ParameterText = param!(
    "landings_required",
    "A selected landing must meet both ends of every flight or run.",
    "An beiden Enden jedes Laufs muss ein gewähltes Podest anschließen.",
);
const MINIMUM_HEADROOM_BELOW: ParameterText = param!(
    "minimum_headroom_below",
    "Least clearance under the flight or ramp above the floors of `headroom_below_spaces`.",
    "Geringste lichte Höhe unter dem Lauf über den Böden der `headroom_below_spaces`.",
);
const HEADROOM_BELOW_SPACES: ParameterText = param!(
    "headroom_below_spaces",
    "The spaces people walk in underneath; declared with `minimum_headroom_below`.",
    "Die darunterliegenden begangenen Räume; zusammen mit `minimum_headroom_below`.",
);
const HANDRAIL_OBJECTS: ParameterText = param!(
    "handrail_objects",
    "Objects that may be handrails, such as railings of a handrail type.",
    "Objekte, die Handläufe sein können, etwa Geländer eines Handlauftyps.",
);
const HANDRAIL_REACH_ACROSS: ParameterText = param!(
    "handrail_reach_across",
    "How far outside the sides a rail may run and still belong to the flight or run.",
    "Wie weit außerhalb der Laufseiten ein Handlauf verlaufen darf und noch zum Lauf gehört.",
);
const HANDRAIL_REACH_ABOVE: ParameterText = param!(
    "handrail_reach_above",
    "How far above the pitch line's highest point a rail's lowest point may lie and still belong to it.",
    "Wie weit über dem höchsten Punkt der Steigungslinie der tiefste Punkt eines Handlaufs liegen darf und er noch dazugehört.",
);
const HANDRAIL_HEIGHT_MINIMUM: ParameterText = param!(
    "handrail_height_minimum",
    "Least height of each rail's top above the pitch line.",
    "Geringste Höhe der Handlaufoberkante über der Steigungslinie.",
);
const HANDRAIL_HEIGHT_MAXIMUM: ParameterText = param!(
    "handrail_height_maximum",
    "Greatest height of each rail's top above the pitch line.",
    "Größte Höhe der Handlaufoberkante über der Steigungslinie.",
);
const HANDRAIL_EXTENSION_MINIMUM: ParameterText = param!(
    "handrail_extension_minimum",
    "Least level extension of the handrail beyond the first and the last nosing or end of the run.",
    "Geringste waagerechte Verlängerung des Handlaufs über die erste und letzte Stufenvorderkante bzw. das Laufende hinaus.",
);
const HANDRAIL_EXTENSION_MAXIMUM: ParameterText = param!(
    "handrail_extension_maximum",
    "Greatest extension of the handrail beyond the first and the last nosing or end of the run.",
    "Größte Verlängerung des Handlaufs über die erste und letzte Stufenvorderkante bzw. das Laufende hinaus.",
);
const HANDRAIL_GAP_MAXIMUM: ParameterText = param!(
    "handrail_gap_maximum",
    "Largest gap in plan between consecutive handrail pieces along a side.",
    "Größte Lücke im Grundriss zwischen aufeinanderfolgenden Handlaufstücken an einer Seite.",
);
const HANDRAIL_SIDES: ParameterText = param!(
    "handrail_sides",
    "`one` or `both`: the sides a handrail must run along.",
    "`one` oder `both`: die Seiten, an denen ein Handlauf verlaufen muss.",
);
const HANDRAIL_BOTH_SIDES_ABOVE_WIDTH: ParameterText = param!(
    "handrail_both_sides_above_width",
    "With `handrail_sides` `one`, a flight or run wider than this needs handrails on both sides.",
    "Bei `handrail_sides` `one` braucht ein Lauf, der breiter ist als dieser Wert, Handläufe auf beiden Seiten.",
);
const LANDING_DOORS: ParameterText = param!(
    "landing_doors",
    "Doors that must not stand on the landing at either end; needs `landing_objects`.",
    "Türen, die nicht auf dem Podest an einem der Enden stehen dürfen; erfordert `landing_objects`.",
);
const LANDING_DOOR_HEIGHT: ParameterText = param!(
    "landing_door_height",
    "Height of the column over a landing that a door's body must not reach into.",
    "Höhe des Raums über einem Podest, in den der Türkörper nicht hineinragen darf.",
);
const LANDING_DOOR_SWING: ParameterText = param!(
    "landing_door_swing",
    "No `landing_doors` door may swing over a landing either.",
    "Zudem darf keine Tür aus `landing_doors` über ein Podest aufschlagen.",
);
const END_SPACE_DEPTH: ParameterText = param!(
    "end_space_depth",
    "Depth along the walking direction of the free space required at each end.",
    "Tiefe der an jedem Ende geforderten freien Bewegungsfläche in Laufrichtung.",
);
const END_SPACE_WIDTH: ParameterText = param!(
    "end_space_width",
    "Width of the free space required at each end.",
    "Breite der an jedem Ende geforderten freien Bewegungsfläche.",
);
const END_SPACE_HEIGHT: ParameterText = param!(
    "end_space_height",
    "Height of the free space required at each end.",
    "Höhe der an jedem Ende geforderten freien Bewegungsfläche.",
);
const END_SPACE_OBSTACLES: ParameterText = param!(
    "end_space_obstacles",
    "Objects that must not reach into the free space at either end.",
    "Objekte, die an keinem Ende in die freie Bewegungsfläche hineinragen dürfen.",
);
const CLEAR_WIDTH_MINIMUM: ParameterText = param!(
    "clear_width_minimum",
    "Least clear width the `clear_width_obstacles` leave across the flight or run within the band.",
    "Geringste lichte Breite, die die `clear_width_obstacles` quer zum Lauf innerhalb des Bands freilassen.",
);
const CLEAR_WIDTH_OBSTACLES: ParameterText = param!(
    "clear_width_obstacles",
    "What narrows the clear width: handrails, walls and anything beside or over the walking surface.",
    "Was die lichte Breite einengt: Handläufe, Wände und alles neben oder über der Lauffläche.",
);
const CLEAR_WIDTH_BAND_FROM: ParameterText = param!(
    "clear_width_band_from",
    "Bottom of the band above the pitch line or surface in which the clear width is measured.",
    "Unterkante des Bands über der Steigungslinie bzw. Lauffläche, in dem die lichte Breite gemessen wird.",
);
const CLEAR_WIDTH_BAND_TO: ParameterText = param!(
    "clear_width_band_to",
    "Top of the band in which the clear width is measured.",
    "Oberkante des Bands, in dem die lichte Breite gemessen wird.",
);

// A host's face and its openings.

const HOST_OPENING_PATH: ParameterText = param!(
    "opening_path",
    "Relationship steps from the host to its openings, such as IfcRelVoidsElement forward.",
    "Beziehungsschritte vom Bauteil zu seinen Öffnungen, etwa IfcRelVoidsElement vorwärts.",
);
const HOST_OPENING_SELECTOR: ParameterText = param!(
    "opening_selector",
    "The openings counted; every reached object by default.",
    "Die berücksichtigten Öffnungen; standardmäßig jedes erreichte Objekt.",
);
const LENGTH_AXIS: ParameterText = param!(
    "length_axis",
    "The host axis along the face's length: `extrusion`, `profile-x` or `profile-y`.",
    "Die Bauteilachse entlang der Länge der Ansichtsfläche: `extrusion`, `profile-x` oder `profile-y`.",
);
const HEIGHT_AXIS: ParameterText = param!(
    "height_axis",
    "The host axis along the face's height, distinct from `length_axis`.",
    "Die Bauteilachse entlang der Höhe der Ansichtsfläche, verschieden von `length_axis`.",
);
const MINIMUM_OPENING_AREA: ParameterText = param!(
    "minimum_opening_area",
    "Openings smaller than this area in the face are left out.",
    "Öffnungen, die in der Ansichtsfläche kleiner als diese Fläche sind, bleiben unberücksichtigt.",
);

// Containers matched across models.

const CONTAINER_RELATIONSHIP: ParameterText = param!(
    "container_relationship",
    "A derived relationship matching containers of different models, such as axioval:derived.same-level; with `level_property`.",
    "Eine abgeleitete Beziehung, die Container verschiedener Modelle zuordnet, etwa axioval:derived.same-level; zusammen mit `level_property`.",
);
const LEVEL_PROPERTY: ParameterText = param!(
    "level_property",
    "The property, such as the storey's elevation or name, by which `container_relationship` matches containers as one level.",
    "Die Eigenschaft, etwa Höhenlage oder Name des Geschosses, über die `container_relationship` Container als eine Ebene zuordnet.",
);

const CATEGORY_PROPERTY: ParameterText = param!(
    "category_property",
    "Property whose value starts each finding of an object in brackets, as its category.",
    "Eigenschaft, deren Wert jedem Befund eines Objekts in Klammern als Kategorie vorangestellt wird.",
);

/// Builds `quantity-takeoff`'s parameters: three group keys, eight measure
/// columns, then the scope and the boundary tolerance.
macro_rules! takeoff_parameters {
    (groups: $($group:literal)*; measures: $($measure:literal)*) => {
        &[
            $(
                param!(
                    concat!("group_", $group),
                    concat!("Group key ", $group, ": the property read on each object, outermost key first."),
                    concat!("Gruppierungsschlüssel ", $group, ": die an jedem Objekt gelesene Eigenschaft, äußerster Schlüssel zuerst."),
                ),
                param!(
                    concat!("group_", $group, "_path"),
                    concat!("Read group key ", $group, " on the objects this relationship path reaches instead."),
                    concat!("Gruppierungsschlüssel ", $group, " stattdessen an den Objekten lesen, die dieser Beziehungspfad erreicht."),
                ),
                param!(
                    concat!("group_", $group, "_name"),
                    concat!("Id of group column ", $group, "; `group_", $group, "` without it."),
                    concat!("Kennung der Gruppenspalte ", $group, "; ohne Angabe `group_", $group, "`."),
                ),
            )*
            $(
                param!(
                    concat!("measure_", $measure),
                    concat!("The property column ", $measure, " reads: a stated number, quantity or text, or a measured value."),
                    concat!("Die Eigenschaft, die Spalte ", $measure, " liest: eine angegebene Zahl, Größe oder ein Text oder ein Messwert."),
                ),
                param!(
                    concat!("measure_", $measure, "_aggregates"),
                    concat!("Aggregates of column ", $measure, ", any of `sum`, `min`, `max`, `mean` and `values`."),
                    concat!("Aggregate der Spalte ", $measure, ", beliebige aus `sum`, `min`, `max`, `mean` und `values`."),
                ),
                param!(
                    concat!("measure_", $measure, "_name"),
                    concat!("Name of column ", $measure, " after its aggregate; derived from the property without it."),
                    concat!("Name der Spalte ", $measure, " nach ihrem Aggregat; ohne Angabe aus der Eigenschaft abgeleitet."),
                ),
                param!(
                    concat!("measure_", $measure, "_kind"),
                    concat!("Kind of column ", $measure, ": `property` (default), `related`, `boundary_area`, `property_set`, `profile` or `computed`."),
                    concat!("Art der Spalte ", $measure, ": `property` (Standard), `related`, `boundary_area`, `property_set`, `profile` oder `computed`."),
                ),
                param!(
                    concat!("measure_", $measure, "_path"),
                    concat!("Relationship path of a `related` column ", $measure, "."),
                    concat!("Beziehungspfad einer `related`-Spalte ", $measure, "."),
                ),
                param!(
                    concat!("measure_", $measure, "_bounding"),
                    concat!("The bounding elements of a `boundary_area` column ", $measure, ", such as walls."),
                    concat!("Die begrenzenden Bauteile einer `boundary_area`-Spalte ", $measure, ", etwa Wände."),
                ),
                param!(
                    concat!("measure_", $measure, "_property_set"),
                    concat!("The property set a `property_set` column ", $measure, " expands."),
                    concat!("Der Eigenschaftssatz, den eine `property_set`-Spalte ", $measure, " aufschlüsselt."),
                ),
                param!(
                    concat!("measure_", $measure, "_expression"),
                    concat!("The expression of a `computed` column ", $measure, " over the other columns."),
                    concat!("Der Ausdruck einer `computed`-Spalte ", $measure, " über die anderen Spalten."),
                ),
                param!(
                    concat!("measure_", $measure, "_unit"),
                    concat!("The unit the values of column ", $measure, " are stated in, such as m2 or EUR/m2."),
                    concat!("Die Einheit, in der die Werte der Spalte ", $measure, " angegeben sind, etwa m2 oder EUR/m2."),
                ),
            )*
            param!(
                "across_sources",
                "One set of groups for the whole project; per source by default.",
                "Eine Gruppierung für das gesamte Projekt; standardmäßig je Quelle.",
            ),
            param!(
                "boundary_plane_tolerance",
                "How far from a face plane of the space's body a boundary surface may lie and still count, for `boundary_area`; 0 by default.",
                "Wie weit eine Begrenzungsfläche von einer Flächenebene des Raumkörpers entfernt liegen darf und noch zählt, für `boundary_area`; standardmäßig 0.",
            ),
        ]
    };
}

/// Sorted by `id`.
pub(crate) const CAPABILITY_TEXTS: &[CapabilityText] = &[
    CapabilityText {
        id: "axioval:capability.accessible-route",
        label: en_de("Accessible route", "Barrierefreier Weg"),
        help: en_de(
            "Checks that each selected destination can be reached from a start through the route spaces by a body of the given width and headroom, through portals and connectors wide enough. An undecided obstacle, or a block that only may exist, leaves the destination not evaluated.",
            "Prüft, ob jedes gewählte Ziel von einem Startpunkt über die Wegeräume für einen Körper der angegebenen Breite und Kopfhöhe erreichbar ist, durch ausreichend breite Durchgänge und Verbindungselemente. Ein unentschiedenes Hindernis oder eine nur mögliche Sperre lässt das Ziel unbewertet.",
        ),
        parameters: &[
            param!(
                "route_selector",
                "The spaces a route may cross.",
                "Die Räume, die ein Weg durchqueren darf.",
            ),
            param!(
                "start_selector",
                "The start points, such as entrances; a destination any start reaches passes.",
                "Die Startpunkte, etwa Eingänge; ein Ziel, das von einem Start erreicht wird, besteht.",
            ),
            param!(
                "portal_selector",
                "The doors and openings a route may pass.",
                "Die Türen und Öffnungen, die ein Weg passieren darf.",
            ),
            param!(
                "lift_selector",
                "The lifts a route may use between levels.",
                "Die Aufzüge, die ein Weg zwischen Ebenen nutzen darf.",
            ),
            param!(
                "ramp_selector",
                "The ramps a route may use between levels.",
                "Die Rampen, die ein Weg zwischen Ebenen nutzen darf.",
            ),
            param!(
                "stair_selector",
                "The stairs between levels, which `forbid_stairs` may rule out.",
                "Die Treppen zwischen Ebenen, die `forbid_stairs` ausschließen kann.",
            ),
            param!(
                "obstacle_selector",
                "What obstructs route spaces and portals within the headroom band, such as furniture.",
                "Was Wegeräume und Durchgänge innerhalb des Kopfhöhenbands versperrt, etwa Möbel.",
            ),
            SUBTRACT_DOOR_SWINGS,
            param!(
                "width_metres",
                "The body's width in metres, the route's least clear width.",
                "Die Körperbreite in Metern, die geringste lichte Breite des Wegs.",
            ),
            param!(
                "clear_height_metres",
                "Headroom band above each floor in metres; each space's own height without it.",
                "Kopfhöhenband über jedem Boden in Metern; ohne Angabe die eigene Höhe jedes Raums.",
            ),
            param!(
                "door_width_metres",
                "Least clear width in metres of every portal on the route.",
                "Geringste lichte Breite jedes Durchgangs auf dem Weg, in Metern.",
            ),
            param!(
                "ramp_width_metres",
                "Least clear width in metres of a ramp on the route.",
                "Geringste lichte Breite einer Rampe auf dem Weg, in Metern.",
            ),
            param!(
                "stair_width_metres",
                "Least clear width in metres of a stair on the route.",
                "Geringste lichte Breite einer Treppe auf dem Weg, in Metern.",
            ),
            param!(
                "forbid_stairs",
                "No route may use a stair; true by default.",
                "Kein Weg darf eine Treppe benutzen; standardmäßig true.",
            ),
            param!(
                "clear_width_property",
                "Where the source states a portal's or connector's clear width, as a length.",
                "Wo die Quelle die lichte Breite eines Durchgangs oder Verbindungselements als Länge angibt.",
            ),
            param!(
                "obstruction_depth_metres",
                "How far in metres obstacles may intrude from a route space's boundary without obstructing it; 0 by default.",
                "Wie weit Hindernisse vom Rand eines Wegeraums hineinragen dürfen, ohne ihn zu versperren, in Metern; standardmäßig 0.",
            ),
            param!(
                "surface_gap_metres",
                "Route spaces at most this far apart, in metres, are joined across the gap; 0 by default.",
                "Wegeräume, die höchstens so weit auseinanderliegen (Meter), werden über die Lücke verbunden; standardmäßig 0.",
            ),
            PASSING_WIDTH_METRES,
            PASSING_LENGTH_METRES,
            PASSING_SPACING_METRES,
            PASSING_REACH_METRES,
        ],
    },
    CapabilityText {
        id: "axioval:capability.allowed-profile",
        label: en_de("Allowed profile", "Zulässiges Profil"),
        help: en_de(
            "Checks that each selected object's body is one swept solid whose profile fits a row of the profile table within the tolerances. A body that is no single swept profile is a finding; a dimension the source refuses leaves the object not evaluated unless another row fits.",
            "Prüft, ob der Körper jedes gewählten Objekts ein einzelner Extrusionskörper ist, dessen Profil innerhalb der Toleranzen einer Zeile der Profiltabelle entspricht. Ein Körper ohne einzelnes Extrusionsprofil ist ein Befund; eine von der Quelle verweigerte Abmessung lässt das Objekt unbewertet, sofern keine andere Zeile passt.",
        ),
        parameters: &[
            param!(
                "profiles",
                "The allowed profiles: a type pattern, an optional name pattern, dimensions and per-row tolerances.",
                "Die zulässigen Profile: ein Typmuster, ein optionales Namensmuster, Abmessungen und zeilenweise Toleranzen.",
            ),
            param!(
                "tolerance",
                "Length every dimension may be off by; exact without it.",
                "Länge, um die jede Abmessung abweichen darf; ohne Angabe exakt.",
            ),
            param!(
                "case_sensitive",
                "Whether type and name patterns respect case; false by default.",
                "Ob Typ- und Namensmuster Groß- und Kleinschreibung beachten; standardmäßig false.",
            ),
            param!(
                "angle_tolerance",
                "Plane angle every slope may be off by; exact without it.",
                "Ebener Winkel, um den jede Neigung abweichen darf; ohne Angabe exakt.",
            ),
            param!(
                "match",
                "`rows` (default): a profile fits one whole row; `per_dimension`: any combination of the listed values fits.",
                "`rows` (Standard): ein Profil passt zu einer ganzen Zeile; `per_dimension`: jede Kombination der aufgeführten Werte passt.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.area-ratio",
        label: en_de("Area ratio", "Flächenverhältnis"),
        help: en_de(
            "Checks at each anchor that the summed areas of the numerator objects over those of the denominator, as footprints, facade areas or stated areas, lie within the bounds. An area straddling a bound, or an undecided member that could change the ratio, leaves the anchor not evaluated.",
            "Prüft für jedes Bezugsobjekt, ob die summierten Flächen der Zählerobjekte im Verhältnis zu denen des Nenners, als Grundrissflächen, Fassadenflächen oder angegebene Flächen, innerhalb der Grenzen liegen. Eine Fläche, die eine Grenze überspannt, oder ein unentschiedenes Mitglied, das das Verhältnis ändern könnte, lässt das Bezugsobjekt unbewertet.",
        ),
        parameters: &[
            param!(
                "numerator_selector",
                "The objects whose areas form the numerator.",
                "Die Objekte, deren Flächen den Zähler bilden.",
            ),
            param!(
                "denominator_selector",
                "The objects whose areas form the denominator; the anchor's own footprint without it.",
                "Die Objekte, deren Flächen den Nenner bilden; ohne Angabe die eigene Grundrissfläche des Bezugsobjekts.",
            ),
            param!(
                "minimum",
                "Least ratio, inclusive.",
                "Kleinstes zulässiges Verhältnis, einschließlich.",
            ),
            param!(
                "maximum",
                "Greatest ratio, inclusive.",
                "Größtes zulässiges Verhältnis, einschließlich.",
            ),
            param!(
                "numerator_property",
                "Area property read on the numerator objects instead of measuring them.",
                "Flächeneigenschaft, die an den Zählerobjekten gelesen wird, statt sie zu messen.",
            ),
            param!(
                "denominator_property",
                "Area property read on the denominator objects instead of measuring them.",
                "Flächeneigenschaft, die an den Nennerobjekten gelesen wird, statt sie zu messen.",
            ),
            param!(
                "measure",
                "`footprint` (default) or `facade`: what both sides measure.",
                "`footprint` (Standard) oder `facade`: was beide Seiten messen.",
            ),
            param!(
                "numerator_measure",
                "`footprint` or `facade` for the numerator alone; not together with `measure`.",
                "`footprint` oder `facade` nur für den Zähler; nicht zusammen mit `measure`.",
            ),
            param!(
                "denominator_measure",
                "`footprint` or `facade` for the denominator alone; not together with `measure`.",
                "`footprint` oder `facade` nur für den Nenner; nicht zusammen mit `measure`.",
            ),
            param!(
                "numerator_derivation",
                "`light-area`: take each numerator member's light-transmitting area from its stated area, the light-area table or the frame allowance.",
                "`light-area`: die lichtdurchlässige Fläche jedes Zählerobjekts aus angegebener Fläche, Lichtflächentabelle oder Rahmenabzug ermitteln.",
            ),
            param!(
                "empty_numerator_finding",
                "Report an anchor that reaches no numerator object instead of judging a ratio of 0.",
                "Ein Bezugsobjekt ohne erreichtes Zählerobjekt melden, statt ein Verhältnis von 0 zu bewerten.",
            ),
            param!(
                "overall_width",
                "With `light-area`: a member's overall width, a length.",
                "Bei `light-area`: die Gesamtbreite eines Objekts, eine Länge.",
            ),
            param!(
                "overall_height",
                "With `light-area`: a member's overall height, a length.",
                "Bei `light-area`: die Gesamthöhe eines Objekts, eine Länge.",
            ),
            param!(
                "light_area_table",
                "Rows of type pattern, width, height and light area, used where no light area is stated.",
                "Zeilen aus Typmuster, Breite, Höhe und Lichtfläche, verwendet, wo keine Lichtfläche angegeben ist.",
            ),
            param!(
                "light_type",
                "The type name the table rows' type patterns match.",
                "Der Typname, mit dem die Typmuster der Tabellenzeilen abgeglichen werden.",
            ),
            param!(
                "light_type_path",
                "Relationship path to the objects `light_type` is read from, such as the type object.",
                "Beziehungspfad zu den Objekten, an denen `light_type` gelesen wird, etwa dem Typobjekt.",
            ),
            param!(
                "light_size_tolerance",
                "Length within which a table row's size matches; exact by default.",
                "Länge, innerhalb derer die Größe einer Tabellenzeile passt; standardmäßig exakt.",
            ),
            param!(
                "frame_width",
                "Width of the frame allowance deducted around the perimeter, a length of at least zero.",
                "Breite des umlaufenden Rahmenabzugs, eine Länge von mindestens null.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.body-extent",
        label: en_de("Body extent", "Körperabmessung"),
        help: en_de(
            "Measures each selected object's body along one of its placement axes and checks the extent against a stated length within a tolerance, or against bounds. An extent straddling the bound, an unplaced object or an unmeasurable body is not evaluated.",
            "Misst den Körper jedes gewählten Objekts entlang einer seiner Platzierungsachsen und prüft die Abmessung gegen eine angegebene Länge innerhalb einer Toleranz oder gegen Grenzen. Eine Abmessung, die die Grenze überspannt, ein nicht platziertes Objekt oder ein nicht messbarer Körper bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "axis",
                "`right`, `forward` or `up`: the placement axis measured along.",
                "`right`, `forward` oder `up`: die Platzierungsachse, entlang der gemessen wird.",
            ),
            param!(
                "target_property",
                "A length the object states that the extent must equal, such as a layer set's total thickness.",
                "Eine vom Objekt angegebene Länge, der die Abmessung entsprechen muss, etwa die Gesamtdicke eines Schichtaufbaus.",
            ),
            param!(
                "tolerance",
                "Length by which the extent may differ from `target_property`; exact without it.",
                "Länge, um die die Abmessung von `target_property` abweichen darf; ohne Angabe exakt.",
            ),
            param!(
                "minimum",
                "Least extent, inclusive, instead of `target_property`.",
                "Kleinste Abmessung, einschließlich, anstelle von `target_property`.",
            ),
            param!(
                "maximum",
                "Greatest extent, inclusive, instead of `target_property`.",
                "Größte Abmessung, einschließlich, anstelle von `target_property`.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.centre-line-distance",
        label: en_de("Centre line distance to walls", "Achsabstand zu Wänden"),
        help: en_de(
            "Checks that the centre line of each selected component's footprint lies within a distance range from the walls beside it, such as a WC's axis from the side wall. A footprint without a unique orientation, or a distance straddling a bound, is not evaluated.",
            "Prüft, ob die Mittelachse des Grundrisses jedes gewählten Ausstattungsgegenstands in einem Abstandsbereich zu den seitlichen Wänden liegt, etwa die Achse eines WCs zur Seitenwand. Ein Grundriss ohne eindeutige Ausrichtung oder ein Abstand, der eine Grenze überspannt, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "wall_selector",
                "The walls or other bodies measured to.",
                "Die Wände oder anderen Körper, zu denen gemessen wird.",
            ),
            param!(
                "centre_line",
                "`long`, `short` or `against-wall`: which axis of the footprint is the centre line.",
                "`long`, `short` oder `against-wall`: welche Achse des Grundrisses die Mittelachse ist.",
            ),
            param!(
                "sides",
                "`nearest`: judge the nearer wall; `both`: judge each side on its own.",
                "`nearest`: die nähere Wand bewerten; `both`: jede Seite einzeln bewerten.",
            ),
            param!(
                "minimum",
                "Least distance from the centre line to a wall.",
                "Geringster Abstand von der Mittelachse zu einer Wand.",
            ),
            param!(
                "maximum",
                "Greatest distance from the centre line to a wall.",
                "Größter Abstand von der Mittelachse zu einer Wand.",
            ),
            param!(
                "reach",
                "How far from the centre line to look for walls; a wall beyond it is none.",
                "Wie weit von der Mittelachse aus nach Wänden gesucht wird; eine Wand darüber hinaus zählt nicht.",
            ),
            param!(
                "inset",
                "How far each side strip is shortened at both ends; 0 by default.",
                "Um wie viel jeder Seitenstreifen an beiden Enden verkürzt wird; standardmäßig 0.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.clash",
        label: en_de("Clash detection", "Kollisionsprüfung"),
        help: en_de(
            "Checks the selected objects against their counterparts for duplicates, bodies inside others, intersections beyond the tolerances and clearance clashes, optionally grouped and graded. A pair whose distance or extent straddles a tolerance is not evaluated.",
            "Prüft die gewählten Objekte gegen ihre Gegenobjekte auf Duplikate, ineinanderliegende Körper, Durchdringungen jenseits der Toleranzen und Abstandskollisionen, optional gruppiert und abgestuft. Ein Paar, dessen Abstand oder Ausdehnung eine Toleranz überspannt, bleibt unbewertet.",
        ),
        parameters: &[
            CLASH_COUNTERPARTS,
            PENETRATION_TOLERANCE_METRES,
            CLEARANCE_METRES,
            DUPLICATE_TOLERANCE_METRES,
            HORIZONTAL_TOLERANCE_METRES,
            VERTICAL_TOLERANCE_METRES,
            VOLUME_TOLERANCE_CUBIC_METRES,
            REPORT_DUPLICATES,
            REPORT_CONTAINMENT,
            REPORT_INTERSECTIONS,
            EXCLUDE_PATHS,
            EXCLUDE_TARGET_PROPERTY,
            EXCLUDE_SAME_LAYER,
            GROUP_BY,
            PER_STOREY,
            CLASH_STOREY_PATH,
            GROUP_PROPERTY,
            GROUP_TOLERANCE_METRES,
            SEVERITY_BY_CLASS,
            GRADE_BY,
            SEVERITY_GRADES,
            DUPLICATE_QUANTITIES,
            TOLERANCE_CASES,
        ],
    },
    CapabilityText {
        id: "axioval:capability.clash-matrix",
        label: en_de("Clash matrix", "Kollisionsmatrix"),
        help: en_de(
            "Judges each clash pair with the tolerances and severity of the single most specific cell of a matrix keyed by discipline, properties and selectors on both sides. Tied cells or an unreadable key leave the pair not evaluated; a pair no cell covers is ignored unless reported.",
            "Bewertet jedes Kollisionspaar mit den Toleranzen und dem Schweregrad der spezifischsten Zelle einer Matrix, die beide Seiten nach Fachdisziplin, Eigenschaften und Selektoren unterscheidet. Gleichrangige Zellen oder ein nicht lesbarer Schlüssel lassen das Paar unbewertet; ein von keiner Zelle erfasstes Paar wird ignoriert, sofern es nicht gemeldet werden soll.",
        ),
        parameters: &[
            CLASH_COUNTERPARTS,
            param!(
                "cells",
                "One row per cell: discipline patterns, selectors and key patterns for both sides of a pair, with that cell's tolerances, switches, severity and label.",
                "Eine Zeile je Zelle: Disziplinmuster, Selektoren und Schlüsselmuster für beide Seiten eines Paars, mit den Toleranzen, Schaltern, dem Schweregrad und der Bezeichnung dieser Zelle.",
            ),
            param!(
                "key_1",
                "The text property the `subject_key_1` and `counterpart_key_1` columns match.",
                "Die Texteigenschaft, mit der die Spalten `subject_key_1` und `counterpart_key_1` abgeglichen werden.",
            ),
            param!(
                "key_2",
                "The text property the `subject_key_2` and `counterpart_key_2` columns match.",
                "Die Texteigenschaft, mit der die Spalten `subject_key_2` und `counterpart_key_2` abgeglichen werden.",
            ),
            param!(
                "key_3",
                "The text property the `subject_key_3` and `counterpart_key_3` columns match.",
                "Die Texteigenschaft, mit der die Spalten `subject_key_3` und `counterpart_key_3` abgeglichen werden.",
            ),
            param!(
                "case_sensitive",
                "Whether patterns respect case; true by default.",
                "Ob Muster Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "symmetric",
                "A cell covers a pair either way round; true by default.",
                "Eine Zelle gilt für ein Paar in beiden Richtungen; standardmäßig true.",
            ),
            param!(
                "report_unmatched",
                "Report a pair no cell covers; false by default.",
                "Ein Paar melden, das keine Zelle abdeckt; standardmäßig false.",
            ),
            param!(
                "exclude_same_system",
                "Skip pairs within one system; true by default.",
                "Paare innerhalb eines Systems überspringen; standardmäßig true.",
            ),
            param!(
                "system_path",
                "Relationship path from an object to its system.",
                "Beziehungspfad von einem Objekt zu seinem System.",
            ),
            EXCLUDE_PATHS,
            EXCLUDE_TARGET_PROPERTY,
            EXCLUDE_SAME_LAYER,
            GROUP_BY,
            PER_STOREY,
            CLASH_STOREY_PATH,
            GROUP_PROPERTY,
            GROUP_TOLERANCE_METRES,
            SEVERITY_BY_CLASS,
            GRADE_BY,
            SEVERITY_GRADES,
            DUPLICATE_QUANTITIES,
            TOLERANCE_CASES,
        ],
    },
    CapabilityText {
        id: "axioval:capability.classification",
        label: en_de("Classification requirement", "Klassifikationsanforderung"),
        help: en_de(
            "Checks that each selected object carries a classification in the required system with the required code, given as literals or patterns, optionally or as prohibited. An assignment without a stated system that could decide the verdict leaves the object not evaluated.",
            "Prüft, ob jedes gewählte Objekt eine Klassifikation im geforderten System mit dem geforderten Code trägt, angegeben als Literale oder Muster, optional oder als verboten. Eine Zuordnung ohne angegebenes System, die das Ergebnis entscheiden könnte, lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "codes",
                "Codes one of which the assignment must carry; the code of any parent entry also counts.",
                "Codes, von denen die Zuordnung einen tragen muss; auch der Code jedes übergeordneten Eintrags zählt.",
            ),
            param!(
                "code_patterns",
                "XML Schema patterns the code must match as a whole.",
                "XML-Schema-Muster, denen der Code vollständig entsprechen muss.",
            ),
            param!(
                "systems",
                "Classification systems one of which the assignment must belong to.",
                "Klassifikationssysteme, zu denen die Zuordnung gehören muss.",
            ),
            param!(
                "system_patterns",
                "XML Schema patterns the system name must match as a whole.",
                "XML-Schema-Muster, denen der Systemname vollständig entsprechen muss.",
            ),
            param!(
                "optional",
                "An object without any classification passes; one with a classification must meet the requirement.",
                "Ein Objekt ohne jede Klassifikation besteht; eines mit Klassifikation muss die Anforderung erfüllen.",
            ),
            param!(
                "prohibited",
                "Invert the verdict: a matching classification is a finding.",
                "Das Ergebnis umkehren: eine passende Klassifikation ist ein Befund.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.component-clearance",
        label: en_de("Component clearance", "Bewegungsfläche an Bauteilen"),
        help: en_de(
            "Checks that a free volume of the declared size lies on a stated side of each selected component, such as a transfer area beside a WC, optionally inside its space and resting on a support. An obstacle that only may intrude, or a position straddling an edge, leaves that side not evaluated.",
            "Prüft, ob auf einer angegebenen Seite jedes gewählten Bauteils ein freies Volumen der angegebenen Größe liegt, etwa eine Umsetzfläche neben einem WC, optional innerhalb seines Raums und auf einer Unterlage. Ein nur möglicherweise hineinragendes Hindernis oder eine Lage, die eine Kante überspannt, lässt diese Seite unbewertet.",
        ),
        parameters: &[
            param!(
                "side",
                "`front`, `back`, `left` or `right` of the component's front; exactly one of `side` and `sides`.",
                "`front`, `back`, `left` oder `right` bezogen auf die Vorderseite des Bauteils; genau eines von `side` und `sides`.",
            ),
            param!(
                "sides",
                "Several sides instead of one, each checked as `side` is.",
                "Mehrere Seiten statt einer, jede wie bei `side` geprüft.",
            ),
            param!(
                "quantifier",
                "With `sides`: `all` (default) checks every side, `any` needs one free side.",
                "Bei `sides`: `all` (Standard) prüft jede Seite, `any` verlangt eine freie Seite.",
            ),
            param!(
                "front_axis",
                "Which way the front faces: `forward`, `-forward`, `right`, `-right`, `stated`, `swing`, `-swing` or `against-wall`.",
                "Wohin die Vorderseite zeigt: `forward`, `-forward`, `right`, `-right`, `stated`, `swing`, `-swing` oder `against-wall`.",
            ),
            param!(
                "both_sides",
                "Also check the opposite side, as its own finding.",
                "Zusätzlich die gegenüberliegende Seite prüfen, als eigenen Befund.",
            ),
            param!(
                "width",
                "Box width across the side; in a component mode, the length added to the component's own width.",
                "Breite des Quaders quer zur Seite; in einem Bauteilmodus die zur Bauteilbreite addierte Länge.",
            ),
            param!(
                "width_mode",
                "`fixed` (default), `component_plus` or `component_clamped`: how the width is sized.",
                "`fixed` (Standard), `component_plus` oder `component_clamped`: wie die Breite bemessen wird.",
            ),
            param!(
                "width_minimum",
                "Lower clamp of the width in a clamped mode.",
                "Untere Begrenzung der Breite in einem begrenzten Modus.",
            ),
            param!(
                "width_maximum",
                "Upper clamp of the width in a clamped mode.",
                "Obere Begrenzung der Breite in einem begrenzten Modus.",
            ),
            param!(
                "depth",
                "Box depth away from the side; in a component mode, the length added to the component's own depth.",
                "Tiefe des Quaders von der Seite weg; in einem Bauteilmodus die zur Bauteiltiefe addierte Länge.",
            ),
            param!(
                "depth_mode",
                "`fixed` (default), `component_plus`, `component_clamped` or `less_clear_width`: how the depth is sized.",
                "`fixed` (Standard), `component_plus`, `component_clamped` oder `less_clear_width`: wie die Tiefe bemessen wird.",
            ),
            param!(
                "depth_minimum",
                "Lower clamp of the depth in a clamped or `less_clear_width` mode.",
                "Untere Begrenzung der Tiefe im begrenzten Modus oder bei `less_clear_width`.",
            ),
            param!(
                "depth_maximum",
                "Upper clamp of the depth in a clamped or `less_clear_width` mode.",
                "Obere Begrenzung der Tiefe im begrenzten Modus oder bei `less_clear_width`.",
            ),
            param!(
                "depth_from",
                "`face` (default): the volume starts at the component's outermost point on the side; `midline`: at its midline.",
                "`face` (Standard): das Volumen beginnt am äußersten Punkt des Bauteils auf der Seite; `midline`: an seiner Mittellinie.",
            ),
            param!(
                "radius",
                "Radius of a cylinder instead of a box.",
                "Radius eines Zylinders anstelle eines Quaders.",
            ),
            param!(
                "height",
                "Height of the volume; required unless `top_datum` is declared.",
                "Höhe des Volumens; erforderlich, sofern `top_datum` nicht angegeben ist.",
            ),
            param!(
                "height_mode",
                "`fixed` (default), `component_plus` or `component_clamped`: how the height is sized.",
                "`fixed` (Standard), `component_plus` oder `component_clamped`: wie die Höhe bemessen wird.",
            ),
            param!(
                "height_minimum",
                "Lower clamp of the height in a clamped mode.",
                "Untere Begrenzung der Höhe in einem begrenzten Modus.",
            ),
            param!(
                "height_maximum",
                "Upper clamp of the height in a clamped mode.",
                "Obere Begrenzung der Höhe in einem begrenzten Modus.",
            ),
            param!(
                "size_mode",
                "`minimum` (default): the volume must be free; `maximum`: no larger volume may be free; `fixed`: both.",
                "`minimum` (Standard): das Volumen muss frei sein; `maximum`: kein größeres Volumen darf frei sein; `fixed`: beides.",
            ),
            param!(
                "size_tolerance",
                "Tolerance of the size; `maximum` and `fixed` need a positive one; 0 by default.",
                "Toleranz der Größe; `maximum` und `fixed` benötigen eine positive; standardmäßig 0.",
            ),
            param!(
                "offset",
                "Gap between the component's outermost point on the side and the volume; 0 by default, negative overlaps.",
                "Abstand zwischen dem äußersten Punkt des Bauteils auf der Seite und dem Volumen; standardmäßig 0, negativ überlappt.",
            ),
            param!(
                "lateral_offset",
                "Moves the volume across the side, to the right as seen looking out of it.",
                "Verschiebt das Volumen quer zur Seite, nach rechts mit Blick von der Seite weg.",
            ),
            param!(
                "align",
                "`centre` (default), `left`, `right`, `handle` or `hinge`: where the volume is aligned across the side.",
                "`centre` (Standard), `left`, `right`, `handle` oder `hinge`: wo das Volumen quer zur Seite ausgerichtet wird.",
            ),
            param!(
                "slide_from",
                "Start of the lateral range in which a floating volume may lie; with `slide_to`.",
                "Beginn des seitlichen Bereichs, in dem ein verschiebbares Volumen liegen darf; zusammen mit `slide_to`.",
            ),
            param!(
                "slide_to",
                "End of the lateral range in which a floating volume may lie.",
                "Ende des seitlichen Bereichs, in dem ein verschiebbares Volumen liegen darf.",
            ),
            param!(
                "depth_slide_from",
                "Start of the range away from the component in which a floating volume may lie; with `depth_slide_to`.",
                "Beginn des Bereichs vom Bauteil weg, in dem ein verschiebbares Volumen liegen darf; zusammen mit `depth_slide_to`.",
            ),
            param!(
                "depth_slide_to",
                "End of the range away from the component in which a floating volume may lie.",
                "Ende des Bereichs vom Bauteil weg, in dem ein verschiebbares Volumen liegen darf.",
            ),
            param!(
                "height_reference",
                "`floor`, `bottom` or `top`: what the volume's base is measured from.",
                "`floor`, `bottom` oder `top`: wovon aus die Unterkante des Volumens gemessen wird.",
            ),
            param!(
                "vertical_offset",
                "Height of the volume's base above the reference; 0 by default.",
                "Höhe der Unterkante des Volumens über dem Bezug; standardmäßig 0.",
            ),
            param!(
                "top_datum",
                "`floor`, `bottom` or `top`: the reference of the volume's top, instead of a height.",
                "`floor`, `bottom` oder `top`: Bezug der Oberkante des Volumens anstelle einer Höhe.",
            ),
            param!(
                "top_offset",
                "Height of the volume's top above `top_datum`; 0 by default.",
                "Höhe der Oberkante des Volumens über `top_datum`; standardmäßig 0.",
            ),
            param!(
                "obstacles",
                "What may obstruct the volume; the component itself never does.",
                "Was das Volumen versperren kann; das Bauteil selbst nie.",
            ),
            param!(
                "allowed_intruders",
                "Objects that may stand in the volume.",
                "Objekte, die im Volumen stehen dürfen.",
            ),
            param!(
                "protrusion",
                "How far an obstacle may reach into the volume through its plan sides.",
                "Wie weit ein Hindernis über die Grundrissseiten in das Volumen hineinragen darf.",
            ),
            param!(
                "within_space",
                "Also require the volume's plan to lie inside the spaces `space_path` reaches.",
                "Zusätzlich verlangen, dass der Grundriss des Volumens innerhalb der über `space_path` erreichten Räume liegt.",
            ),
            param!(
                "space_path",
                "Relationship steps from the component to its spaces.",
                "Beziehungsschritte vom Bauteil zu seinen Räumen.",
            ),
            param!(
                "wall_selector",
                "With `against-wall`: the walls a component may stand against.",
                "Bei `against-wall`: die Wände, vor denen ein Bauteil stehen kann.",
            ),
            param!(
                "wall_reach",
                "With `against-wall`: how far from the footprint's centre lines to look for walls.",
                "Bei `against-wall`: wie weit von den Mittelachsen des Grundrisses aus nach Wänden gesucht wird.",
            ),
            param!(
                "wall_inset",
                "With `against-wall`: how far each side strip is shortened at both edges; 0 by default.",
                "Bei `against-wall`: um wie viel jeder Seitenstreifen an beiden Rändern verkürzt wird; standardmäßig 0.",
            ),
            param!(
                "support_selector",
                "Bodies whose tops must carry the volume's whole plan, such as a slab or landing.",
                "Körper, deren Oberseiten den gesamten Grundriss des Volumens tragen müssen, etwa eine Decke oder ein Podest.",
            ),
            param!(
                "support_tolerance",
                "How far from the volume's base a supporting top may lie; with `support_selector`.",
                "Wie weit eine tragende Oberseite von der Unterkante des Volumens entfernt liegen darf; zusammen mit `support_selector`.",
            ),
            param!(
                "clear_width_property",
                "With `less_clear_width`: the door's stated clear width, a length.",
                "Bei `less_clear_width`: die angegebene lichte Breite der Tür, eine Länge.",
            ),
            CLEAR_WIDTH_FROM_LEAVES,
            OVERALL_WIDTH,
            WIDTH_DEDUCTION,
        ],
    },
    CapabilityText {
        id: "axioval:capability.component-visibility",
        label: en_de("Component visibility", "Sichtbeziehung von Bauteilen"),
        help: en_de(
            "Checks from an eye above each selected component that at least a number of targets within a radius are in view, or that none is. A target only grazed, or hidden only with an undecided blocker's help, is undecided and may leave the component not evaluated.",
            "Prüft von einem Augpunkt über jedem gewählten Bauteil, ob mindestens eine Anzahl von Zielen innerhalb eines Radius sichtbar ist oder keines. Ein nur gestreiftes oder nur durch ein unentschiedenes Hindernis verdecktes Ziel ist unentschieden und kann das Bauteil unbewertet lassen.",
        ),
        parameters: &[
            param!(
                "targets",
                "What must, or must not, be seen, such as doors.",
                "Was gesehen bzw. nicht gesehen werden darf, etwa Türen.",
            ),
            param!(
                "blockers",
                "What may hide a target, such as walls and columns.",
                "Was ein Ziel verdecken kann, etwa Wände und Stützen.",
            ),
            param!(
                "eye_height",
                "Height of the eye above the component's base.",
                "Augenhöhe über der Unterkante des Bauteils.",
            ),
            param!(
                "radius",
                "Only targets whose nearest point lies within this distance of the eye count.",
                "Nur Ziele, deren nächster Punkt innerhalb dieses Abstands vom Auge liegt, zählen.",
            ),
            param!(
                "mode",
                "`at-least`: enough targets must be in view; `none`: no target may be in view.",
                "`at-least`: genügend Ziele müssen sichtbar sein; `none`: kein Ziel darf sichtbar sein.",
            ),
            param!(
                "minimum",
                "With `at-least`, how many targets must be in view; 1 by default.",
                "Bei `at-least` die Anzahl der Ziele, die sichtbar sein müssen; standardmäßig 1.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.consistent-value",
        label: en_de("Consistent value", "Einheitlicher Wert"),
        help: en_de(
            "Checks that objects of one kind sharing a key value also share their value, within one source or scope; numbers may agree within a tolerance. A value the tolerance does not apply to, or a range that may lie on either side of it, leaves the group not evaluated.",
            "Prüft, ob Objekte derselben Art mit gleichem Schlüsselwert auch denselben Wert haben, innerhalb einer Quelle oder eines Bereichs; Zahlen dürfen innerhalb einer Toleranz übereinstimmen. Ein Wert, für den die Toleranz nicht gilt, oder eine Spanne, die auf beiden Seiten liegen kann, lässt die Gruppe unbewertet.",
        ),
        parameters: &[
            param!(
                "key",
                "Property whose value forms the groups.",
                "Eigenschaft, deren Wert die Gruppen bildet.",
            ),
            param!(
                "value",
                "Property that must agree within each group.",
                "Eigenschaft, die innerhalb jeder Gruppe übereinstimmen muss.",
            ),
            param!(
                "case_sensitive",
                "Whether text compares by case; false by default.",
                "Ob Text Groß- und Kleinschreibung unterscheidet; standardmäßig false.",
            ),
            param!(
                "same_kind",
                "Group only objects of one kind; true by default.",
                "Nur Objekte derselben Art gruppieren; standardmäßig true.",
            ),
            param!(
                "across_sources",
                "Form groups across every source of the project; per source by default.",
                "Gruppen über alle Quellen des Projekts bilden; standardmäßig je Quelle.",
            ),
            param!(
                "tolerance",
                "Number the range of a group's numeric values may span; quantities in their SI unit.",
                "Zahl, die die Spanne der Zahlenwerte einer Gruppe umfassen darf; Größen in ihrer SI-Einheit.",
            ),
            param!(
                "tolerance_quantity",
                "Quantity the range may span, applied only to values of its dimension.",
                "Größe, die die Spanne umfassen darf, nur auf Werte ihrer Dimension angewandt.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.containment",
        label: en_de("Containment", "Enthaltensein"),
        help: en_de(
            "Checks that the selected inner elements lie in their outer counterparts by a minimum share of volume, optionally with cover distances to the outer faces and counts per outer element. Undecided containments, straddling intervals and counts undecided elements could change are not evaluated.",
            "Prüft, ob die gewählten inneren Bauteile mit einem Mindestvolumenanteil in ihren äußeren Gegenbauteilen liegen, optional mit Überdeckungsabständen zu den äußeren Flächen und Anzahlen je äußerem Bauteil. Unentschiedenes Enthaltensein, überspannende Intervalle und Anzahlen, die unentschiedene Bauteile ändern könnten, bleiben unbewertet.",
        ),
        parameters: &[
            param!(
                "counterparts",
                "The outer elements the selected ones must lie in.",
                "Die äußeren Bauteile, in denen die gewählten liegen müssen.",
            ),
            param!(
                "minimum_volume_ratio",
                "Least share of the smaller body's volume the two must share.",
                "Kleinster Anteil am Volumen des kleineren Körpers, den beide gemeinsam haben müssen.",
            ),
            param!(
                "combine_adjacent",
                "Also take outer elements whose surfaces meet together.",
                "Äußere Bauteile, deren Oberflächen sich berühren, gemeinsam betrachten.",
            ),
            param!(
                "cover",
                "Rows bounding the distance from the inner body to a class of the outer element's faces (top, side, bottom or any).",
                "Zeilen, die den Abstand vom inneren Körper zu einer Flächenklasse des äußeren Bauteils begrenzen (oben, Seite, unten oder beliebig).",
            ),
            param!(
                "minimum_count",
                "Least number of inner elements each outer element holds.",
                "Kleinste Anzahl innerer Bauteile je äußerem Bauteil.",
            ),
            param!(
                "maximum_count",
                "Greatest number of inner elements each outer element holds.",
                "Größte Anzahl innerer Bauteile je äußerem Bauteil.",
            ),
            param!(
                "report_orphans",
                "Report an inner element lying in no outer element.",
                "Ein inneres Bauteil melden, das in keinem äußeren liegt.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.coordinate-consistency",
        label: en_de("Coordinate consistency", "Koordinatenkonsistenz"),
        help: en_de(
            "Checks that every source shares the reference source's coordinate system: world frame, true north, map conversion and site placement within the tolerances. A statement only one source makes, or an unreadable coordinate system, leaves the source not evaluated.",
            "Prüft, ob jede Quelle das Koordinatensystem der Referenzquelle teilt: Weltkoordinatensystem, geografischen Norden, Georeferenzierung und Lage des Grundstücks innerhalb der Toleranzen. Eine Angabe, die nur eine Quelle macht, oder ein nicht lesbares Koordinatensystem lässt die Quelle unbewertet.",
        ),
        parameters: &[
            param!(
                "reference",
                "Discipline of the reference source; the first source in identity order without it.",
                "Fachdisziplin der Referenzquelle; ohne Angabe die erste Quelle in Identitätsreihenfolge.",
            ),
            param!(
                "length_tolerance",
                "Length tolerance in metres; 0.001 by default.",
                "Längentoleranz in Metern; standardmäßig 0,001.",
            ),
            param!(
                "angle_tolerance",
                "Angle tolerance in degrees; 0.01 by default.",
                "Winkeltoleranz in Grad; standardmäßig 0,01.",
            ),
            param!(
                "scale_tolerance",
                "Absolute scale tolerance; 0 by default.",
                "Absolute Maßstabstoleranz; standardmäßig 0.",
            ),
            param!(
                "require_map_conversion",
                "A source without a map conversion is a finding instead of not evaluated.",
                "Eine Quelle ohne Georeferenzierung ist ein Befund statt unbewertet.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.corridor-end-openings",
        label: en_de("Openings at corridor ends", "Öffnungen an Flurenden"),
        help: en_de(
            "Finds the windows or other selected openings sitting in the wall a selected corridor ends at. An end whose wall cannot be decided, or a gap or facing length straddling its threshold, leaves the opening not evaluated.",
            "Findet die Fenster oder anderen gewählten Öffnungen in der Stirnwand, an der ein gewählter Flur endet. Ein Ende, dessen Wand nicht bestimmt werden kann, oder ein Abstand bzw. eine Überdeckungslänge, die ihren Schwellenwert überspannt, lässt die Öffnung unbewertet.",
        ),
        parameters: &[
            param!(
                "opening_path",
                "Relationship path from the corridor to its openings.",
                "Beziehungspfad vom Flur zu seinen Öffnungen.",
            ),
            param!(
                "opening_selector",
                "Which reached objects are checked, such as windows.",
                "Welche erreichten Objekte geprüft werden, etwa Fenster.",
            ),
            param!(
                "wall_depth",
                "How far in metres an opening may lie from the end wall's face and still sit in it; 0.5 by default.",
                "Wie weit eine Öffnung von der Stirnwandfläche entfernt liegen darf und noch in ihr sitzt, in Metern; standardmäßig 0,5.",
            ),
            param!(
                "facing",
                "How much of the end wall in metres an opening must face to sit in it; more than 0.1 by default.",
                "Wie viel der Stirnwand eine Öffnung gegenüberliegen muss, um in ihr zu sitzen, in Metern; standardmäßig mehr als 0,1.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.counterpart-coverage",
        label: en_de("Counterpart coverage", "Deckung durch Gegenbauteile"),
        help: en_de(
            "Checks how much of each selected element its counterparts leave uncovered in plan and in height, or in its elevation, and grades the uncovered share by severity thresholds. A share straddling the lowest threshold, or a counterpart that may only possibly cover, leaves the element not evaluated.",
            "Prüft, wie viel jedes gewählten Bauteils seine Gegenbauteile im Grundriss und in der Höhe oder in seiner Ansicht ungedeckt lassen, und stuft den ungedeckten Anteil nach Schwellenwerten ab. Ein Anteil, der den niedrigsten Schwellenwert überspannt, oder ein nur möglicherweise deckendes Gegenbauteil lässt das Bauteil unbewertet.",
        ),
        parameters: &[
            param!(
                "counterparts",
                "The objects that should cover each element, such as another discipline's walls.",
                "Die Objekte, die jedes Bauteil decken sollen, etwa die Wände einer anderen Fachdisziplin.",
            ),
            param!(
                "tolerance",
                "One growth for both checks, a length; a negative one switches the checks off.",
                "Ein gemeinsamer Zuschlag für beide Prüfungen, eine Länge; ein negativer schaltet die Prüfungen ab.",
            ),
            param!(
                "horizontal_tolerance",
                "Growth in plan, declared with `vertical_tolerance` instead of `tolerance`; negative switches the plan check off.",
                "Zuschlag im Grundriss, zusammen mit `vertical_tolerance` statt `tolerance`; negativ schaltet die Grundrissprüfung ab.",
            ),
            param!(
                "vertical_tolerance",
                "Growth in height, declared with `horizontal_tolerance`; negative switches the height check off.",
                "Zuschlag in der Höhe, zusammen mit `horizontal_tolerance`; negativ schaltet die Höhenprüfung ab.",
            ),
            param!(
                "info_above",
                "Uncovered share in [0, 1) above which a finding is information.",
                "Ungedeckter Anteil in [0, 1), oberhalb dessen ein Befund ein Hinweis ist.",
            ),
            param!(
                "warning_above",
                "Uncovered share in [0, 1) above which a finding is a warning.",
                "Ungedeckter Anteil in [0, 1), oberhalb dessen ein Befund eine Warnung ist.",
            ),
            param!(
                "error_above",
                "Uncovered share in [0, 1) above which a finding is an error.",
                "Ungedeckter Anteil in [0, 1), oberhalb dessen ein Befund ein Fehler ist.",
            ),
            param!(
                "axis_tolerance",
                "Only counterparts whose long axis lies within this angle of parallel to the element's count; in [0°, 45°).",
                "Nur Gegenbauteile, deren Längsachse innerhalb dieses Winkels parallel zu der des Bauteils liegt, zählen; in [0°, 45°).",
            ),
            param!(
                "measure",
                "`plan_and_height` (default) or `elevation`: one check in the element's own elevation plane.",
                "`plan_and_height` (Standard) oder `elevation`: eine Prüfung in der Ansichtsebene des Bauteils.",
            ),
            param!(
                "infill_counterparts",
                "With `elevation`: frame members, such as columns and beams, whose infill covers when enough is uncovered.",
                "Bei `elevation`: Rahmenbauteile wie Stützen und Träger, deren Ausfachung deckt, wenn genug ungedeckt bleibt.",
            ),
            param!(
                "infill_above",
                "Uncovered share above which the frame's infill covers; in [0, 1), 0.5 by default.",
                "Ungedeckter Anteil, oberhalb dessen die Ausfachung des Rahmens deckt; in [0, 1), standardmäßig 0,5.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.distance",
        label: en_de("Distance", "Abstand"),
        help: en_de(
            "Measures distances from the selected objects to their counterparts, in space, in plan or vertically, and bounds the nearest one, every one or a count of them. A counterpart whose distance straddles a bound counts as unknown, and a verdict it could change is not evaluated.",
            "Misst Abstände von den gewählten Objekten zu ihren Gegenobjekten, räumlich, im Grundriss oder vertikal, und begrenzt den nächsten, jeden oder eine Anzahl davon. Ein Gegenobjekt, dessen Abstand eine Grenze überspannt, gilt als unbekannt, und ein Ergebnis, das es ändern könnte, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "counterparts",
                "The objects distances are measured to.",
                "Die Objekte, zu denen Abstände gemessen werden.",
            ),
            param!(
                "minimum_metres",
                "Least distance in metres; may be computed per object.",
                "Kleinster Abstand in Metern; kann je Objekt berechnet werden.",
            ),
            param!(
                "maximum_metres",
                "Greatest distance in metres; may be computed per object.",
                "Größter Abstand in Metern; kann je Objekt berechnet werden.",
            ),
            param!(
                "mode",
                "`nearest` (default), `none_closer_than` or `at_least`: which counterparts are bounded.",
                "`nearest` (Standard), `none_closer_than` oder `at_least`: welche Gegenobjekte begrenzt werden.",
            ),
            param!(
                "count",
                "With `at_least`: how many counterparts must lie within `maximum_metres`.",
                "Bei `at_least`: wie viele Gegenobjekte innerhalb von `maximum_metres` liegen müssen.",
            ),
            param!(
                "projection",
                "`minimum_3d` (default), `horizontal`, `vertical` or `plan_overlap`: how the distance is measured.",
                "`minimum_3d` (Standard), `horizontal`, `vertical` oder `plan_overlap`: wie der Abstand gemessen wird.",
            ),
            param!(
                "footprint_offset_metres",
                "With `vertical`: how far in metres the footprints are grown in plan.",
                "Bei `vertical`: um wie viel die Grundrisse vergrößert werden, in Metern.",
            ),
            param!(
                "vertical_direction",
                "With `vertical`: `either` (default), `above` or `below`.",
                "Bei `vertical`: `either` (Standard), `above` oder `below`.",
            ),
            param!(
                "subject_extent",
                "`leaf_swing`: measure the subject by the plan area its leaves or panels sweep instead of its body.",
                "`leaf_swing`: das Objekt über die von seinen Flügeln überstrichene Grundrissfläche statt über seinen Körper messen.",
            ),
            param!(
                "counterpart_extent",
                "`leaf_swing`: measure counterparts by the plan area their leaves or panels sweep instead of their bodies.",
                "`leaf_swing`: Gegenobjekte über die von ihren Flügeln überstrichene Grundrissfläche statt über ihren Körper messen.",
            ),
            param!(
                "subject_surface",
                "With `vertical`: the subject's `top` or `bottom` surface measured from.",
                "Bei `vertical`: die Oberfläche `top` oder `bottom` des Objekts, von der gemessen wird.",
            ),
            param!(
                "counterpart_surface",
                "With `vertical`: the counterpart's `top`, `bottom` or `nearest` surface measured to.",
                "Bei `vertical`: die Oberfläche `top`, `bottom` oder `nearest` des Gegenobjekts, zu der gemessen wird.",
            ),
            param!(
                "elevation_overlap",
                "`overlapping`: with `horizontal`, only counterparts at the subject's heights count.",
                "`overlapping`: bei `horizontal` zählen nur Gegenobjekte auf der Höhe des Objekts.",
            ),
            param!(
                "elevation_offset_metres",
                "How far in metres the subject's heights are widened for `elevation_overlap`.",
                "Um wie viel die Höhen des Objekts für `elevation_overlap` erweitert werden, in Metern.",
            ),
            param!(
                "container_selector",
                "With a traversal, only shared containers of this kind scope the counterparts.",
                "Bei einer Beziehungsangabe begrenzen nur gemeinsame Container dieser Art die Gegenobjekte.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.door-swing",
        label: en_de("Door swing", "Türaufschlag"),
        help: en_de(
            "Checks that each selected door swings into, or not into, the spaces it opens onto, such as a WC door opening outward. A door without a hinged leaf or with unreadable leaves is not evaluated, and a refused probe decides only what it cannot change.",
            "Prüft, ob jede gewählte Tür in die Räume, zu denen sie öffnet, aufschlägt oder nicht aufschlägt, etwa eine WC-Tür nach außen. Eine Tür ohne Drehflügel oder mit nicht lesbaren Türblättern bleibt unbewertet, und eine verweigerte Prüfung entscheidet nur, was sie nicht ändern kann.",
        ),
        parameters: &[
            param!(
                "space_path",
                "Relationship steps from a door to the spaces it opens onto.",
                "Beziehungsschritte von einer Tür zu den Räumen, zu denen sie öffnet.",
            ),
            param!(
                "swing_into",
                "Among the reached spaces it picks, the door must swing into at least one.",
                "Unter den erreichten, hiervon gewählten Räumen muss die Tür in mindestens einen aufschlagen.",
            ),
            param!(
                "swing_not_into",
                "The door must swing into none of the reached spaces it picks.",
                "Die Tür darf in keinen der erreichten, hiervon gewählten Räume aufschlagen.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.effective-coverage",
        label: en_de("Effective coverage", "Abdeckung durch Wirkungsbereiche"),
        help: en_de(
            "Checks that the union of the sources' effect areas covers enough of each selected element's footprint, as sprinklers, detectors or extinguishers reach a room, optionally with a capacity check. A covered share straddling the minimum is not evaluated.",
            "Prüft, ob die Vereinigung der Wirkungsbereiche der Wirkquellen einen ausreichenden Anteil des Grundrisses jedes gewählten Bauteils abdeckt, wie Sprinkler, Melder oder Feuerlöscher einen Raum erreichen, optional mit Kapazitätsprüfung. Ein abgedeckter Anteil, der das Minimum überspannt, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "sources",
                "The objects whose effect areas cover, such as sprinklers.",
                "Die Wirkquellen, deren Wirkungsbereiche abdecken, etwa Sprinkler.",
            ),
            param!(
                "mode",
                "`grown`, `touching`, `travel` or `visible`: how far an effect reaches.",
                "`grown`, `touching`, `travel` oder `visible`: wie weit eine Wirkung reicht.",
            ),
            param!(
                "range",
                "The effect's range, a length.",
                "Die Reichweite der Wirkung, eine Länge.",
            ),
            param!(
                "minimum_ratio",
                "Share of the footprint in (0, 1] that must be covered.",
                "Anteil des Grundrisses in (0, 1], der abgedeckt sein muss.",
            ),
            param!(
                "blockers",
                "With `travel` and `visible`: objects whose footprints travel and sight must go round.",
                "Bei `travel` und `visible`: Objekte, deren Grundrisse Weg und Sicht umgehen müssen.",
            ),
            param!(
                "touch_tolerance",
                "With `touching`: how far apart in plan a source may stand and still touch; 0 by default.",
                "Bei `touching`: wie weit eine Wirkquelle im Grundriss entfernt stehen darf und noch berührt; standardmäßig 0.",
            ),
            param!(
                "capacity_property",
                "Property summed over the sources reaching the element for the capacity check.",
                "Eigenschaft, die für die Kapazitätsprüfung über die erreichenden Wirkquellen summiert wird.",
            ),
            param!(
                "capacity_multiplier",
                "Factor applied to each source's capacity before the sum is compared with the area.",
                "Faktor für die Kapazität jeder Wirkquelle, bevor die Summe mit der Fläche verglichen wird.",
            ),
            param!(
                "capacity_multiplier_property",
                "Each source's own multiplier, read on it, instead of `capacity_multiplier`.",
                "Der eigene Faktor jeder Wirkquelle, an ihr gelesen, anstelle von `capacity_multiplier`.",
            ),
            param!(
                "area_property",
                "The element's stated area, a number in square metres or an area, instead of its footprint's.",
                "Die angegebene Fläche des Bauteils, eine Zahl in Quadratmetern oder eine Fläche, anstelle der Grundrissfläche.",
            ),
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.empty-host",
        label: en_de("Empty host", "Vollständig durchbrochenes Bauteil"),
        help: en_de(
            "Reports each selected host, such as a wall, whose openings void its whole face. An opening that cannot be placed, openings that may overlap or an opening of undecided selection leave the host not evaluated.",
            "Meldet jedes gewählte Bauteil, etwa eine Wand, dessen Öffnungen seine gesamte Ansichtsfläche durchbrechen. Eine nicht platzierbare Öffnung, möglicherweise überlappende Öffnungen oder eine Öffnung mit unentschiedener Auswahl lassen das Bauteil unbewertet.",
        ),
        parameters: &[
            HOST_OPENING_PATH,
            HOST_OPENING_SELECTOR,
            LENGTH_AXIS,
            HEIGHT_AXIS,
            MINIMUM_OPENING_AREA,
            param!(
                "area_tolerance",
                "Face area the openings may leave uncovered and still void the host; 0 by default.",
                "Ansichtsfläche, die die Öffnungen freilassen dürfen und das Bauteil dennoch durchbrechen; standardmäßig 0.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.escape-route",
        label: en_de("Escape route", "Rettungsweg"),
        help: en_de(
            "Checks each selected space against its use row: travel distance to the nearest exit, number of exits or independent routes, and widths of exits, doors and passages for its occupant load. Anything an unmeasured walk or an undecided object could change is not evaluated.",
            "Prüft jeden gewählten Raum gegen seine Nutzungszeile: Rettungsweglänge bis zum nächsten Ausgang, Anzahl der Ausgänge oder unabhängigen Rettungswege sowie Breiten von Ausgängen, Türen und Fluren für seine Personenzahl. Alles, was ein nicht gemessener Weg oder ein unentschiedenes Objekt ändern könnte, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "uses",
                "Rows per use: the spaces, maximum travel, route start, exits needed and area per occupant.",
                "Zeilen je Nutzung: die Räume, maximale Rettungsweglänge, Wegbeginn, erforderliche Ausgänge und Fläche je Person.",
            ),
            param!(
                "widths",
                "Rows by occupant load: least clear widths of each exit, all exits, passages and doors in metres.",
                "Zeilen nach Personenzahl: geringste lichte Breiten je Ausgang, aller Ausgänge, der Flure und Türen in Metern.",
            ),
            param!(
                "exit_path",
                "Relationship path from a space to its exits.",
                "Beziehungspfad von einem Raum zu seinen Ausgängen.",
            ),
            param!(
                "exit_selector",
                "Which reached objects are exits.",
                "Welche erreichten Objekte Ausgänge sind.",
            ),
            param!(
                "door_path",
                "Relationship path from a space to its own doors.",
                "Beziehungspfad von einem Raum zu seinen eigenen Türen.",
            ),
            param!(
                "door_selector",
                "Which reached objects are the space's doors and openings.",
                "Welche erreichten Objekte die Türen und Öffnungen des Raums sind.",
            ),
            param!(
                "clear_width_property",
                "An exit's or door's stated clear width, a length.",
                "Die angegebene lichte Breite eines Ausgangs oder einer Tür, eine Länge.",
            ),
            param!(
                "walking_height",
                "Headroom in metres of the walking body; required by `maximum_travel`.",
                "Kopfhöhe des gehenden Körpers in Metern; erforderlich für `maximum_travel`.",
            ),
            param!(
                "walking_step",
                "Largest step in metres the walk crosses; required by `maximum_travel`.",
                "Größte überschreitbare Stufe in Metern; erforderlich für `maximum_travel`.",
            ),
            param!(
                "sections",
                "Rows of route sections, such as stairs, each metre walked on which counts `factor` times.",
                "Zeilen von Wegabschnitten, etwa Treppen, auf denen jeder begangene Meter `factor`-fach zählt.",
            ),
            param!(
                "section_path",
                "Relationship path from a space to the shared sections it uses; with `shared_by`.",
                "Beziehungspfad von einem Raum zu den gemeinsam genutzten Abschnitten; zusammen mit `shared_by`.",
            ),
            param!(
                "passage_path",
                "Relationship path from a space to the passages it relies on.",
                "Beziehungspfad von einem Raum zu den Fluren, auf die er angewiesen ist.",
            ),
            param!(
                "passage_selector",
                "The passages, such as corridors, whose widths are checked for the summed load.",
                "Die Flure, deren Breiten für die summierte Personenzahl geprüft werden.",
            ),
            param!(
                "passage_width_property",
                "A passage's stated clear width, a length.",
                "Die angegebene lichte Breite eines Flurs, eine Länge.",
            ),
            param!(
                "exit_door_direction",
                "Every exit door must open in the direction of escape, out of the space.",
                "Jede Ausgangstür muss in Fluchtrichtung aus dem Raum heraus aufschlagen.",
            ),
            param!(
                "walked_passages",
                "Derive each space's passages from the walks out of its doors instead of `passage_path`.",
                "Die Flure jedes Raums aus den Wegen von seinen Türen ableiten statt aus `passage_path`.",
            ),
            param!(
                "no_escape_selector",
                "Objects not usable for escape, such as locked doors: never an exit or start, and every walk keeps out of them.",
                "Für die Flucht nicht nutzbare Objekte, etwa verschlossene Türen: nie Ausgang oder Start, und jeder Weg meidet sie.",
            ),
            param!(
                "compartment_selector",
                "The fire compartments; travel ends where it leaves the start's compartment.",
                "Die Brandabschnitte; der Weg endet dort, wo er den Brandabschnitt des Starts verlässt.",
            ),
            param!(
                "compartment_path",
                "Relationship path from a space to the compartments it lies in.",
                "Beziehungspfad von einem Raum zu den Brandabschnitten, in denen er liegt.",
            ),
            param!(
                "compartment_overlap",
                "A space lies in each compartment covering at least this share, above 0 and at most 1, of its footprint.",
                "Ein Raum liegt in jedem Brandabschnitt, der mindestens diesen Anteil (über 0, höchstens 1) seines Grundrisses überdeckt.",
            ),
            param!(
                "zones",
                "Rows ranking zones; every walk keeps out of whatever ranks above its start.",
                "Zeilen mit Rangstufen von Zonen; jeder Weg meidet alles, was über seinem Start rangiert.",
            ),
            param!(
                "exit_count",
                "`exits` (default) counts exits; `routes` counts independent routes.",
                "`exits` (Standard) zählt Ausgänge; `routes` zählt unabhängige Rettungswege.",
            ),
            param!(
                "route_door_selector",
                "The doors on routes whose summed occupant loads are checked against `door_width`.",
                "Die Türen auf Rettungswegen, deren summierte Personenzahl gegen `door_width` geprüft wird.",
            ),
            param!(
                "common_path_factor",
                "How many times a metre of the common path counts; at least 1.",
                "Wie oft ein Meter des gemeinsamen Wegabschnitts zählt; mindestens 1.",
            ),
            param!(
                "route_door_direction",
                "Every single-swing door the walks cross must open along them.",
                "Jede einflügelige Drehtür auf den Wegen muss in Wegrichtung aufschlagen.",
            ),
            param!(
                "minimum_clear_height",
                "Least clear height in metres of every door, opening and space the walks cross, and of the space itself.",
                "Geringste lichte Höhe in Metern jeder Tür, Öffnung und jedes Raums auf den Wegen sowie des Raums selbst.",
            ),
            param!(
                "clear_height_property",
                "A door's stated clear height, a length.",
                "Die angegebene lichte Höhe einer Tür, eine Länge.",
            ),
            OVERALL_HEIGHT,
            LINING_THICKNESS,
            THRESHOLD_THICKNESS,
            STAIR_SELECTOR,
            RAMP_SELECTOR,
            LIFT_SELECTOR,
            STAIR_LENGTH,
            VERTICAL_FACTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.exit-separation",
        label: en_de("Exit separation", "Abstand der Ausgänge"),
        help: en_de(
            "Checks that each selected space's exits lie at least a fraction of its longest plan diagonal apart, a smaller fraction where a flag such as sprinkler protection is true. A pair or flag that cannot be decided leaves the space not evaluated unless the verdict holds either way.",
            "Prüft, ob die Ausgänge jedes gewählten Raums mindestens einen Anteil seiner längsten Grundrissdiagonale auseinanderliegen, einen kleineren Anteil, wo eine Kennung wie Sprinklerschutz wahr ist. Ein nicht entscheidbares Paar oder eine nicht entscheidbare Kennung lässt den Raum unbewertet, sofern das Ergebnis nicht in jedem Fall feststeht.",
        ),
        parameters: &[
            param!(
                "exit_path",
                "Relationship path from the space to its exits.",
                "Beziehungspfad vom Raum zu seinen Ausgängen.",
            ),
            param!(
                "exit_selector",
                "Which reached objects are exits.",
                "Welche erreichten Objekte Ausgänge sind.",
            ),
            param!(
                "fraction",
                "Share of the diagonal the exits must lie apart; 0.5 by default.",
                "Anteil der Diagonale, um den die Ausgänge auseinanderliegen müssen; standardmäßig 0,5.",
            ),
            param!(
                "flag",
                "Boolean property selecting `flagged_fraction`, such as a sprinkler flag.",
                "Boolesche Eigenschaft, die `flagged_fraction` auswählt, etwa eine Sprinklerkennung.",
            ),
            param!(
                "flag_path",
                "Where `flag` is read: the objects this path reaches from the space; the space itself without it.",
                "Wo `flag` gelesen wird: die Objekte, die dieser Pfad vom Raum aus erreicht; ohne Angabe der Raum selbst.",
            ),
            param!(
                "flagged_fraction",
                "Share of the diagonal that applies when the flag is true.",
                "Anteil der Diagonale, der gilt, wenn die Kennung wahr ist.",
            ),
            param!(
                "flag_sources",
                "Places the flag is read, in order: property set, property and path.",
                "Orte, an denen die Kennung gelesen wird, in Reihenfolge: Eigenschaftssatz, Eigenschaft und Pfad.",
            ),
            param!(
                "flag_default",
                "The flag's value when no source states one.",
                "Der Wert der Kennung, wenn keine Quelle einen angibt.",
            ),
            param!(
                "separation",
                "`closest` (default), `centres` or `farthest`: how the separation of two exits is measured.",
                "`closest` (Standard), `centres` oder `farthest`: wie der Abstand zweier Ausgänge gemessen wird.",
            ),
            param!(
                "pairs",
                "`any` (default): some pair of exits must lie far enough apart; `all`: every pair.",
                "`any` (Standard): ein Ausgangspaar muss weit genug auseinanderliegen; `all`: jedes Paar.",
            ),
            param!(
                "minimum_exits",
                "Fewer exits than this is a finding.",
                "Weniger Ausgänge als diese Anzahl sind ein Befund.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.expression",
        label: en_de("Expression requirement", "Ausdrucksanforderung"),
        help: en_de(
            "Requires an expression over properties, measured values and the rule's own parameters to hold for each selected object. An expression that cannot be decided, such as a measured value straddling a bound, leaves the object not evaluated.",
            "Verlangt, dass ein Ausdruck über Eigenschaften, Messwerte und die eigenen Parameter der Regel für jedes gewählte Objekt erfüllt ist. Ein nicht entscheidbarer Ausdruck, etwa ein Messwert, der eine Grenze überspannt, lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "requirement",
                "The truth expression each object must satisfy.",
                "Der Wahrheitsausdruck, den jedes Objekt erfüllen muss.",
            ),
            param!(
                "deviation",
                "Number expression evaluated for a failing object to grade it by severity bands.",
                "Zahlenausdruck, der für ein nicht erfüllendes Objekt ausgewertet wird, um es nach Schweregradbändern abzustufen.",
            ),
            param!(
                "message",
                "Message replacing the default; `{label}` shows the value of the subexpression with that label.",
                "Meldung anstelle der Standardmeldung; `{label}` zeigt den Wert des Teilausdrucks mit dieser Bezeichnung.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.external-wall-validation",
        label: en_de(
            "External wall validation",
            "Prüfung der Außenwandkennzeichnung",
        ),
        help: en_de(
            "Compares the objects a model declares external with those on the building envelope derived from geometry and reports each selected object that disagrees. An object declaring neither, or whose body cannot be measured, is not evaluated.",
            "Vergleicht die im Modell als außenliegend gekennzeichneten Objekte mit denen auf der geometrisch abgeleiteten Gebäudehülle und meldet jedes gewählte Objekt, bei dem beides abweicht. Ein Objekt ohne Kennzeichnung oder mit nicht messbarem Körper bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "derivations",
                "`all-spaces`, `gross-area-groups` or both: how the envelope is derived, each reported on its own.",
                "`all-spaces`, `gross-area-groups` oder beides: wie die Gebäudehülle abgeleitet wird, jeweils getrennt gemeldet.",
            ),
            param!(
                "bounding_selector",
                "The objects the `all-spaces` envelope is derived around, such as every space.",
                "Die Objekte, um die die Hülle für `all-spaces` abgeleitet wird, etwa alle Räume.",
            ),
            param!(
                "gross_area_group_selector",
                "The gross-area groups the `gross-area-groups` envelope is derived from.",
                "Die Bruttoflächengruppen, aus denen die Hülle für `gross-area-groups` abgeleitet wird.",
            ),
            param!(
                "gross_area_group_path",
                "Relationship steps from each group to its members.",
                "Beziehungsschritte von jeder Gruppe zu ihren Mitgliedern.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.free-floor-circle",
        label: en_de("Free floor circle", "Freie Bewegungsfläche (Kreis)"),
        help: en_de(
            "Checks that each selected space can hold a free circle of the given diameter and height on its floor, optionally reachable from its entrances. Missing services, outages or incomplete evidence leave the space not evaluated.",
            "Prüft, ob jeder gewählte Raum auf seinem Boden einen freien Kreis des angegebenen Durchmessers und der angegebenen Höhe aufnehmen kann, optional erreichbar von seinen Zugängen. Fehlende Dienste, Ausfälle oder unvollständige Nachweise lassen den Raum unbewertet.",
        ),
        parameters: &[
            param!(
                "diameter_metres",
                "Diameter of the circle in metres.",
                "Durchmesser des Kreises in Metern.",
            ),
            param!(
                "height_metres",
                "Free height above the floor in metres.",
                "Freie Höhe über dem Boden in Metern.",
            ),
            FREE_FLOOR_OBSTACLES,
            FREE_FLOOR_BAND_FROM,
            FREE_FLOOR_BAND_TO,
            FREE_FLOOR_MERGE_PATH,
            SUBTRACT_DOOR_SWINGS,
            FREE_FLOOR_ENTRANCE_PATH_WIDTH,
            FREE_FLOOR_ENTRANCE_TOLERANCE,
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.free-floor-rectangle",
        label: en_de("Free floor rectangle", "Freie Bewegungsfläche (Rechteck)"),
        help: en_de(
            "Checks that each selected space can hold a free rectangle of the given size and height on its floor in any rotation, optionally reachable from its entrances. An orientation no service can ground, missing services or incomplete evidence leave the space not evaluated.",
            "Prüft, ob jeder gewählte Raum auf seinem Boden ein freies Rechteck der angegebenen Größe und Höhe in beliebiger Drehung aufnehmen kann, optional erreichbar von seinen Zugängen. Eine von keinem Dienst unterstützte Ausrichtung, fehlende Dienste oder unvollständige Nachweise lassen den Raum unbewertet.",
        ),
        parameters: &[
            param!(
                "width_metres",
                "Width of the rectangle in metres.",
                "Breite des Rechtecks in Metern.",
            ),
            param!(
                "length_metres",
                "Length of the rectangle in metres.",
                "Länge des Rechtecks in Metern.",
            ),
            param!(
                "height_metres",
                "Free height above the floor in metres.",
                "Freie Höhe über dem Boden in Metern.",
            ),
            param!(
                "orientation",
                "Which rotations count; only `any` is supported.",
                "Welche Drehungen zählen; nur `any` wird unterstützt.",
            ),
            FREE_FLOOR_OBSTACLES,
            FREE_FLOOR_BAND_FROM,
            FREE_FLOOR_BAND_TO,
            FREE_FLOOR_MERGE_PATH,
            SUBTRACT_DOOR_SWINGS,
            FREE_FLOOR_ENTRANCE_PATH_WIDTH,
            FREE_FLOOR_ENTRANCE_TOLERANCE,
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.group-composition",
        label: en_de("Group composition", "Zusammensetzung von Gruppen"),
        help: en_de(
            "Checks that each selected group, such as an apartment, holds the members its requirement rows list, allocating members to entries by a maximum matching. An undecided membership, member key or group key leaves the group not evaluated.",
            "Prüft, ob jede gewählte Gruppe, etwa eine Wohnung, die in den Anforderungszeilen aufgeführten Mitglieder enthält, wobei Mitglieder den Einträgen über eine maximale Zuordnung zugewiesen werden. Eine unentschiedene Mitgliedschaft, ein unentschiedener Mitglieds- oder Gruppenschlüssel lässt die Gruppe unbewertet.",
        ),
        parameters: &[
            param!(
                "requirements",
                "Member entries: key patterns, group patterns, a label and the required count.",
                "Mitgliedereinträge: Schlüsselmuster, Gruppenmuster, eine Bezeichnung und die geforderte Anzahl.",
            ),
            param!(
                "key_1",
                "Member property the `key_1` patterns match.",
                "Mitgliedseigenschaft, mit der die Muster `key_1` abgeglichen werden.",
            ),
            param!(
                "key_2",
                "Member property the `key_2` patterns match.",
                "Mitgliedseigenschaft, mit der die Muster `key_2` abgeglichen werden.",
            ),
            param!(
                "key_3",
                "Member property the `key_3` patterns match.",
                "Mitgliedseigenschaft, mit der die Muster `key_3` abgeglichen werden.",
            ),
            param!(
                "case_sensitive",
                "Whether patterns respect case; true by default.",
                "Ob Muster Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "group_key",
                "Older name of `group_key_1`.",
                "Älterer Name von `group_key_1`.",
            ),
            param!(
                "group_key_1",
                "Group property the `group` patterns match, such as an apartment's type.",
                "Gruppeneigenschaft, mit der die Muster `group` abgeglichen werden, etwa der Wohnungstyp.",
            ),
            param!(
                "group_key_2",
                "Group property the `group_2` patterns match.",
                "Gruppeneigenschaft, mit der die Muster `group_2` abgeglichen werden.",
            ),
            param!(
                "group_key_3",
                "Group property the `group_3` patterns match.",
                "Gruppeneigenschaft, mit der die Muster `group_3` abgeglichen werden.",
            ),
            param!(
                "report_absent_groups",
                "Report each requirement row no selected group matches.",
                "Jede Anforderungszeile melden, zu der keine gewählte Gruppe passt.",
            ),
            param!(
                "member_selector",
                "Which reached objects are members; every one by default.",
                "Welche erreichten Objekte Mitglieder sind; standardmäßig jedes.",
            ),
            param!(
                "ungrouped_selector",
                "Objects that must belong to a group; each no selected group reaches is a finding.",
                "Objekte, die zu einer Gruppe gehören müssen; jedes, das keine gewählte Gruppe erreicht, ist ein Befund.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.horizontal-guard",
        label: en_de("Fall protection at edges", "Absturzsicherung an Kanten"),
        help: en_de(
            "Checks that the exposed edges of the selected walking surfaces are guarded by barriers tall and close enough, or by a short fall onto a landing wide enough, and that no climbable object beside a barrier defeats it. A role selector that cannot decide an object leaves the rule not evaluated.",
            "Prüft, ob die offenen Kanten der gewählten Laufflächen durch ausreichend hohe und nahe Brüstungen oder Geländer gesichert sind oder durch eine geringe Absturzhöhe auf eine ausreichend breite Fläche, und ob kein besteigbares Objekt neben einer Absturzsicherung sie unwirksam macht. Ein Rollenselektor, der ein Objekt nicht entscheiden kann, lässt die Regel unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum_barrier_height_metres",
                "Least height in metres of a barrier above the walking surface.",
                "Geringste Höhe einer Brüstung oder eines Geländers über der Lauffläche, in Metern.",
            ),
            param!(
                "maximum_barrier_gap_metres",
                "Largest horizontal gap in metres between the edge and a barrier that still guards it.",
                "Größter waagerechter Abstand in Metern zwischen Kante und einer Absturzsicherung, die sie noch sichert.",
            ),
            param!(
                "maximum_platform_gap_metres",
                "Largest horizontal distance in metres from the edge at which a barrier counts as standing at it.",
                "Größter waagerechter Abstand in Metern von der Kante, bei dem eine Absturzsicherung noch als an ihr stehend gilt.",
            ),
            param!(
                "maximum_landing_gap_metres",
                "Largest horizontal gap in metres between the edge and a lower surface that catches a fall.",
                "Größter waagerechter Abstand in Metern zwischen der Kante und einer tieferen Fläche, die einen Absturz auffängt.",
            ),
            param!(
                "maximum_fall_height_metres",
                "Largest drop in metres onto a lower surface that needs no barrier.",
                "Größte Absturzhöhe in Metern auf eine tiefere Fläche, die keine Absturzsicherung erfordert.",
            ),
            param!(
                "minimum_landing_width_metres",
                "Least width in metres of a lower surface to stand on.",
                "Geringste Breite in Metern einer tieferen Fläche, um darauf zu stehen.",
            ),
            param!(
                "climbable_barrier_distance_metres",
                "Distance in metres from a barrier within which a climbable object defeats it.",
                "Abstand in Metern von einer Absturzsicherung, innerhalb dessen ein besteigbares Objekt sie unwirksam macht.",
            ),
            param!(
                "maximum_climbable_height_metres",
                "Objects whose top lies at most this high, in metres, can be climbed.",
                "Objekte, deren Oberkante höchstens so hoch liegt (Meter), gelten als besteigbar.",
            ),
            param!(
                "minimum_climbable_side_length_metres",
                "Least side length in metres of an object's top for it to be stood on.",
                "Geringste Seitenlänge in Metern der Oberseite eines Objekts, damit man darauf stehen kann.",
            ),
            param!(
                "measure_barrier_from_curb",
                "Measure a barrier standing on a curb from the curb's top instead of the walking surface.",
                "Eine Absturzsicherung auf einer Aufkantung ab deren Oberkante statt ab der Lauffläche messen.",
            ),
            param!(
                "barrier_selector",
                "Objects that may act as barriers; any nearby body without it.",
                "Objekte, die als Absturzsicherung wirken können; ohne Angabe jeder nahe Körper.",
            ),
            param!(
                "landing_selector",
                "Objects that may catch a fall as lower surfaces; any nearby body without it.",
                "Objekte, die als tiefere Fläche einen Absturz auffangen können; ohne Angabe jeder nahe Körper.",
            ),
            param!(
                "climbable_selector",
                "Objects that may be climbed; any nearby body without it.",
                "Objekte, die bestiegen werden können; ohne Angabe jeder nahe Körper.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.keyed-limit",
        label: en_de("Keyed limit", "Grenzwert nach Schlüsseln"),
        help: en_de(
            "Looks up the limit row keyed by facts about each selected object and checks one of its quantities against it: an area, a property, a sill height, a door's clear width or height, a threshold step, a glazing ratio or a measured value. No matching row is a finding; an unknown key a row could need, or tied rows, leave the object not evaluated.",
            "Sucht die Grenzwertzeile, die Merkmale jedes gewählten Objekts als Schlüssel trifft, und prüft eine seiner Größen dagegen: eine Fläche, eine Eigenschaft, eine Brüstungshöhe, lichte Breite oder Höhe einer Tür, eine Schwellenhöhe, einen Verglasungsanteil oder einen Messwert. Keine passende Zeile ist ein Befund; ein unbekannter Schlüssel, den eine Zeile benötigen könnte, oder gleichrangige Zeilen lassen das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "limits",
                "Limit rows: patterns over the keys, and `other_side` for a pair key, with a minimum and a maximum.",
                "Grenzwertzeilen: Muster über die Schlüssel, und `other_side` für einen Paarschlüssel, mit Minimum und Maximum.",
            ),
            param!(
                "quantity",
                "What is limited: `plan-area`, `member-plan-area`, `property`, `sill-height`, `clear-width`, `clear-height`, `threshold-step`, `glazing-ratio` or `measured`.",
                "Was begrenzt wird: `plan-area`, `member-plan-area`, `property`, `sill-height`, `clear-width`, `clear-height`, `threshold-step`, `glazing-ratio` oder `measured`.",
            ),
            param!(
                "quantity_property",
                "The stated value the quantity reads, such as a property, clear width or glazing fraction.",
                "Der angegebene Wert, den die Größe liest, etwa eine Eigenschaft, lichte Breite oder ein Verglasungsanteil.",
            ),
            param!(
                "measured_value",
                "With `measured`: the registered measured value limited, with its parameters.",
                "Bei `measured`: der begrenzte registrierte Messwert mit seinen Parametern.",
            ),
            param!(
                "floor_path",
                "With `sill-height` and `threshold-step`: relationship steps to the floors measured from.",
                "Bei `sill-height` und `threshold-step`: Beziehungsschritte zu den Böden, von denen gemessen wird.",
            ),
            OVERALL_WIDTH,
            WIDTH_DEDUCTION,
            CLEAR_WIDTH_FROM_LEAVES,
            OVERALL_HEIGHT,
            LINING_THICKNESS,
            param!(
                "threshold_thickness",
                "The threshold thickness the door states: deducted from its overall height, and added to its bottom for `threshold-step`.",
                "Die von der Tür angegebene Schwellendicke: von der Gesamthöhe abgezogen und bei `threshold-step` zur Unterkante addiert.",
            ),
            param!(
                "ramp_selector",
                "With `threshold-step`: ramps that may be a side's floor; declared with `ramp_reach`.",
                "Bei `threshold-step`: Rampen, die auf einer Seite der Boden sein können; zusammen mit `ramp_reach`.",
            ),
            param!(
                "ramp_reach",
                "How far in plan from the door a ramp may lie and still be its floor.",
                "Wie weit eine Rampe im Grundriss von der Tür entfernt liegen darf und noch ihr Boden ist.",
            ),
            param!(
                "case_sensitive",
                "Whether key patterns respect case; true by default.",
                "Ob Schlüsselmuster Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "member_selector",
                "With `member-plan-area`: the members whose footprints are summed.",
                "Bei `member-plan-area`: die Mitglieder, deren Grundrissflächen summiert werden.",
            ),
            param!(
                "pair_key",
                "Which key, `key_1` to `key_4`, is read as the unordered pair of spaces on the object's two faces.",
                "Welcher Schlüssel, `key_1` bis `key_4`, als ungeordnetes Paar der Räume auf beiden Seiten des Objekts gelesen wird.",
            ),
            param!(
                "door_type_defaults",
                "Rows of defaults per door type, used only where the door states no value.",
                "Zeilen mit Vorgabewerten je Türtyp, nur verwendet, wo die Tür keinen Wert angibt.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
            param!(
                "key_1",
                "First key, a property matched against the rows' `key_1` patterns.",
                "Erster Schlüssel, eine Eigenschaft, die mit den Mustern `key_1` der Zeilen abgeglichen wird.",
            ),
            param!(
                "key_1_path",
                "Relationship steps to the objects `key_1` is read from; the object itself without it.",
                "Beziehungsschritte zu den Objekten, an denen `key_1` gelesen wird; ohne Angabe das Objekt selbst.",
            ),
            param!(
                "key_2",
                "Second key, a property matched against the rows' `key_2` patterns.",
                "Zweiter Schlüssel, eine Eigenschaft, die mit den Mustern `key_2` der Zeilen abgeglichen wird.",
            ),
            param!(
                "key_2_path",
                "Relationship steps to the objects `key_2` is read from; the object itself without it.",
                "Beziehungsschritte zu den Objekten, an denen `key_2` gelesen wird; ohne Angabe das Objekt selbst.",
            ),
            param!(
                "key_3",
                "Third key, a property matched against the rows' `key_3` patterns.",
                "Dritter Schlüssel, eine Eigenschaft, die mit den Mustern `key_3` der Zeilen abgeglichen wird.",
            ),
            param!(
                "key_3_path",
                "Relationship steps to the objects `key_3` is read from; the object itself without it.",
                "Beziehungsschritte zu den Objekten, an denen `key_3` gelesen wird; ohne Angabe das Objekt selbst.",
            ),
            param!(
                "key_4",
                "Fourth key, a property matched against the rows' `key_4` patterns.",
                "Vierter Schlüssel, eine Eigenschaft, die mit den Mustern `key_4` der Zeilen abgeglichen wird.",
            ),
            param!(
                "key_4_path",
                "Relationship steps to the objects `key_4` is read from; the object itself without it.",
                "Beziehungsschritte zu den Objekten, an denen `key_4` gelesen wird; ohne Angabe das Objekt selbst.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.level-spacing",
        label: en_de("Level spacing", "Geschosshöhen"),
        help: en_de(
            "Checks each level's height, the rise of its elevation to the next level up, against bounds and the prevailing height, and optionally the heights and elevations of its spaces. A height that is not measured or straddles a bound is not evaluated.",
            "Prüft die Höhe jeder Ebene, den Anstieg ihrer Höhenlage bis zur nächsthöheren Ebene, gegen Grenzen und die vorherrschende Höhe, optional auch Höhen und Höhenlagen ihrer Räume. Eine nicht gemessene oder eine Grenze überspannende Höhe bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "member_selector",
                "The levels, such as storeys, reached from each anchor.",
                "Die Ebenen, etwa Geschosse, die von jedem Bezugsobjekt erreicht werden.",
            ),
            param!(
                "order",
                "Length property ordering the levels, such as their elevation.",
                "Längeneigenschaft, die die Ebenen ordnet, etwa ihre Höhenlage.",
            ),
            param!("minimum", "Least level height.", "Geringste Geschosshöhe."),
            param!("maximum", "Greatest level height.", "Größte Geschosshöhe."),
            param!(
                "consistent",
                "Every level's height must match the prevailing one within `tolerance`.",
                "Jede Geschosshöhe muss innerhalb von `tolerance` der vorherrschenden entsprechen.",
            ),
            param!(
                "tolerance",
                "Allowed difference from the prevailing height, a length.",
                "Zulässige Abweichung von der vorherrschenden Höhe, eine Länge.",
            ),
            param!(
                "ignore_lowest",
                "Leave out the lowest level, such as a basement.",
                "Die unterste Ebene auslassen, etwa ein Untergeschoss.",
            ),
            param!(
                "ignore_highest",
                "Leave out the highest level, whose height is otherwise not evaluated unless measured from its contents.",
                "Die oberste Ebene auslassen, deren Höhe sonst unbewertet bleibt, sofern sie nicht aus ihrem Inhalt gemessen wird.",
            ),
            param!(
                "content_path",
                "Relationship steps from a level to its contents, to measure the highest level from their tops.",
                "Beziehungsschritte von einer Ebene zu ihrem Inhalt, um die oberste Ebene aus dessen Oberkanten zu messen.",
            ),
            param!(
                "content_selector",
                "Which contents count for the highest level's height, such as walls.",
                "Welche Inhalte für die Höhe der obersten Ebene zählen, etwa Wände.",
            ),
            param!(
                "space_selector",
                "Spaces whose heights or elevations are compared with their level.",
                "Räume, deren Höhen oder Höhenlagen mit ihrer Ebene verglichen werden.",
            ),
            param!(
                "space_path",
                "Relationship steps from a level to its spaces.",
                "Beziehungsschritte von einer Ebene zu ihren Räumen.",
            ),
            param!(
                "space_tolerance",
                "Allowed difference of a space's height or elevation, a length.",
                "Zulässige Abweichung der Höhe oder Höhenlage eines Raums, eine Länge.",
            ),
            param!(
                "space_height",
                "Compare each space's height with its level's; true by default.",
                "Die Höhe jedes Raums mit der seiner Ebene vergleichen; standardmäßig true.",
            ),
            param!(
                "space_elevation",
                "`bottom`, `top` or `both`: a level's spaces must share that elevation within `space_tolerance`.",
                "`bottom`, `top` oder `both`: die Räume einer Ebene müssen diese Höhenlage innerhalb von `space_tolerance` teilen.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.light-well",
        label: en_de("Light well", "Lichthof"),
        help: en_de(
            "Checks that the spaces stacked into each selected light well are contiguous and share a plan section of sufficient area and width for the well's height. A value straddling a bound, or a tessellated space, leaves the well not evaluated.",
            "Prüft, ob die übereinander gestapelten Räume jedes gewählten Lichthofs lückenlos anschließen und einen gemeinsamen Grundrissquerschnitt mit ausreichender Fläche und Breite für die Höhe des Lichthofs haben. Ein Wert, der eine Grenze überspannt, oder ein tesselierter Raum lässt den Lichthof unbewertet.",
        ),
        parameters: &[
            param!(
                "member_path",
                "Relationship steps from the well to its spaces.",
                "Beziehungsschritte vom Lichthof zu seinen Räumen.",
            ),
            param!(
                "requirements",
                "Rows of maximum height, least section area and least section width; the first row holding the well's height applies.",
                "Zeilen aus maximaler Höhe, geringster Querschnittsfläche und geringster Querschnittsbreite; es gilt die erste Zeile, die die Höhe des Lichthofs umfasst.",
            ),
            param!(
                "gap_tolerance_metres",
                "Largest vertical gap in metres between consecutive spaces; 0 by default.",
                "Größte vertikale Lücke in Metern zwischen aufeinanderfolgenden Räumen; standardmäßig 0.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.local-circulation",
        label: en_de("Local circulation", "Bewegungsflächen im Raum"),
        help: en_de(
            "Checks within each selected space that a path of the given width leads from its entrances to its components, with free areas at the path ends and passing spaces along it, and optionally that it has entrances wide enough. An undecided obstacle, or a circulation map the service refuses, leaves the space not evaluated.",
            "Prüft innerhalb jedes gewählten Raums, ob ein Weg der angegebenen Breite von seinen Zugängen zu seinen Ausstattungsgegenständen führt, mit freien Flächen an den Wegenden und Begegnungsflächen entlang des Wegs, und optional, ob er ausreichend breite Zugänge hat. Ein unentschiedenes Hindernis oder eine vom Dienst verweigerte Bewegungskarte lässt den Raum unbewertet.",
        ),
        parameters: &[
            param!(
                "component_selector",
                "The components the path must reach, such as a WC, bed or washbasin.",
                "Die Ausstattungsgegenstände, die der Weg erreichen muss, etwa WC, Bett oder Waschtisch.",
            ),
            param!(
                "space_path",
                "Relationship steps from a component to the spaces it stands in.",
                "Beziehungsschritte von einem Ausstattungsgegenstand zu den Räumen, in denen er steht.",
            ),
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
            param!(
                "obstacles",
                "What occupies floor; every other object by default, the space's entrances never.",
                "Was Bodenfläche belegt; standardmäßig jedes andere Objekt, die Zugänge des Raums nie.",
            ),
            SUBTRACT_DOOR_SWINGS,
            param!(
                "width_metres",
                "Width of the path in metres.",
                "Breite des Wegs in Metern.",
            ),
            param!(
                "clear_height_metres",
                "Height in metres above the floor to which the path, its end areas and passing spaces must be free.",
                "Höhe über dem Boden in Metern, bis zu der Weg, Endflächen und Begegnungsflächen frei sein müssen.",
            ),
            param!(
                "tolerance_metres",
                "How much farther than half the width an entrance or component may lie from the path, in metres; 0.05 by default.",
                "Wie viel weiter als die halbe Breite ein Zugang oder Ausstattungsgegenstand vom Weg entfernt liegen darf, in Metern; standardmäßig 0,05.",
            ),
            param!(
                "component_mode",
                "`touch` (default): reach each component from an entrance; `link`: link a space's components; `link_sets`: link each with a partner.",
                "`touch` (Standard): jeden Gegenstand von einem Zugang erreichen; `link`: die Gegenstände eines Raums verbinden; `link_sets`: jeden mit einem Partner verbinden.",
            ),
            param!(
                "end_width_metres",
                "Width across the path of the free area each path end needs, in metres.",
                "Breite der an jedem Wegende benötigten freien Fläche quer zum Weg, in Metern.",
            ),
            param!(
                "end_length_metres",
                "Length along the path of the free area each path end needs, in metres.",
                "Länge der an jedem Wegende benötigten freien Fläche entlang des Wegs, in Metern.",
            ),
            param!(
                "end_reach_metres",
                "How far from the end the free area's centre may lie, in metres; half its larger side by default.",
                "Wie weit der Mittelpunkt der freien Fläche vom Wegende entfernt liegen darf, in Metern; standardmäßig ihre halbe größere Seite.",
            ),
            param!(
                "short_end_metres",
                "An end whose branch is shorter than this, in metres, needs no free area.",
                "Ein Wegende, dessen Ast kürzer als dieser Wert in Metern ist, braucht keine freie Fläche.",
            ),
            param!(
                "narrow_end_metres",
                "An end where the free width is less than this, in metres, needs no free area.",
                "Ein Wegende, an dem die freie Breite unter diesem Wert in Metern liegt, braucht keine freie Fläche.",
            ),
            param!(
                "merge_path",
                "Relationship path to the spaces mapped together with the selected one as one walkable area.",
                "Beziehungspfad zu den Räumen, die zusammen mit dem gewählten als eine begehbare Fläche betrachtet werden.",
            ),
            param!(
                "band_from_metres",
                "Where the obstacle band starts above the floor, in metres; the floor by default.",
                "Wo das Hindernisband über dem Boden beginnt, in Metern; standardmäßig am Boden.",
            ),
            param!(
                "end_exempt_selector",
                "Objects near which a path end needs no free area; with `end_exempt_reach_metres`.",
                "Objekte, in deren Nähe ein Wegende keine freie Fläche braucht; zusammen mit `end_exempt_reach_metres`.",
            ),
            param!(
                "end_exempt_reach_metres",
                "How near in metres to such an object a path end is exempt.",
                "Wie nah in Metern ein Wegende einem solchen Objekt sein muss, um ausgenommen zu sein.",
            ),
            param!(
                "partner_selector",
                "With `link_sets`: the objects each component must be linked with.",
                "Bei `link_sets`: die Objekte, mit denen jeder Ausstattungsgegenstand verbunden sein muss.",
            ),
            param!(
                "require_entrances",
                "A space no door or opening reaches is a finding; false by default.",
                "Ein Raum, den keine Tür oder Öffnung erreicht, ist ein Befund; standardmäßig false.",
            ),
            param!(
                "check_entrance_width",
                "An entrance whose clear width is below `width_metres` is a finding; false by default.",
                "Ein Zugang, dessen lichte Breite unter `width_metres` liegt, ist ein Befund; standardmäßig false.",
            ),
            param!(
                "clear_width_property",
                "With `check_entrance_width`: an entrance's stated clear width, a length.",
                "Bei `check_entrance_width`: die angegebene lichte Breite eines Zugangs, eine Länge.",
            ),
            CLEAR_WIDTH_FROM_LEAVES,
            OVERALL_WIDTH,
            WIDTH_DEDUCTION,
            PASSING_WIDTH_METRES,
            PASSING_LENGTH_METRES,
            PASSING_SPACING_METRES,
            PASSING_REACH_METRES,
        ],
    },
    CapabilityText {
        id: "axioval:capability.manual-issue",
        label: en_de("Manual check", "Manuelle Prüfung"),
        help: en_de(
            "Raises the declared title once per rule for a check owed by hand, against the first selected object and relating the others; a decidedly empty selection raises it against the project. Nothing is judged, and undecided objects are reported not evaluated.",
            "Meldet den angegebenen Titel einmal je Regel für eine von Hand zu erbringende Prüfung, am ersten gewählten Objekt und mit Verweis auf die übrigen; eine sicher leere Auswahl meldet ihn am Projekt. Es wird nichts bewertet, und unentschiedene Objekte werden als unbewertet gemeldet.",
        ),
        parameters: &[
            param!(
                "title",
                "Title of the issue raised.",
                "Titel des gemeldeten Punkts.",
            ),
            param!(
                "description",
                "Longer description of what to check.",
                "Ausführlichere Beschreibung dessen, was zu prüfen ist.",
            ),
            param!(
                "category",
                "Category of the issue.",
                "Kategorie des gemeldeten Punkts.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.model-comparison",
        label: en_de("Model comparison", "Modellvergleich"),
        help: en_de(
            "Compares the base model with the revised model of the run and reports added, removed and changed objects, properties, placements, geometry and relationships. A change involving only undecided or unmatched objects is not evaluated.",
            "Vergleicht das Ausgangsmodell mit dem überarbeiteten Modell des Laufs und meldet hinzugefügte, entfernte und geänderte Objekte, Eigenschaften, Platzierungen, Geometrie und Beziehungen. Eine Änderung, die nur unentschiedene oder nicht zugeordnete Objekte betrifft, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "base",
                "Discipline of the base model.",
                "Fachdisziplin des Ausgangsmodells.",
            ),
            param!(
                "revised",
                "Discipline of the revised model.",
                "Fachdisziplin des überarbeiteten Modells.",
            ),
            param!(
                "identity_scheme",
                "External identity scheme objects are matched by, such as `ifc-globalid`.",
                "Externes Identitätsschema, über das Objekte zugeordnet werden, etwa `ifc-globalid`.",
            ),
            param!(
                "identity_property",
                "Property whose value matches objects, such as a door number.",
                "Eigenschaft, deren Wert Objekte zuordnet, etwa eine Türnummer.",
            ),
            param!(
                "revised_identity_property",
                "Property read on the revised model instead; needs `identity_property`.",
                "Eigenschaft, die stattdessen im überarbeiteten Modell gelesen wird; erfordert `identity_property`.",
            ),
            param!(
                "match_by",
                "Order of the matchers: `identity`, `property`, `geometry`, `placement`, `overlap` and `related`.",
                "Reihenfolge der Zuordnungsverfahren: `identity`, `property`, `geometry`, `placement`, `overlap` und `related`.",
            ),
            param!(
                "properties",
                "Rows of property set and property compared value by value.",
                "Zeilen aus Eigenschaftssatz und Eigenschaft, die Wert für Wert verglichen werden.",
            ),
            param!(
                "property_sets",
                "Property sets compared whole.",
                "Eigenschaftssätze, die vollständig verglichen werden.",
            ),
            param!(
                "all_property_sets",
                "Compare every property set whole.",
                "Jeden Eigenschaftssatz vollständig vergleichen.",
            ),
            param!(
                "compare_placement",
                "Compare the objects' placements.",
                "Die Platzierungen der Objekte vergleichen.",
            ),
            param!(
                "compare_geometry",
                "Compare the objects' geometry by their bounds.",
                "Die Geometrie der Objekte über ihre Begrenzungen vergleichen.",
            ),
            param!(
                "compare_coordinate_systems",
                "Compare the two models' coordinate systems.",
                "Die Koordinatensysteme beider Modelle vergleichen.",
            ),
            param!(
                "length_tolerance",
                "Length tolerance in metres of the comparison; 0 by default.",
                "Längentoleranz des Vergleichs in Metern; standardmäßig 0.",
            ),
            param!(
                "angle_tolerance",
                "Angle tolerance in degrees of the comparison; 0 by default.",
                "Winkeltoleranz des Vergleichs in Grad; standardmäßig 0.",
            ),
            param!(
                "revised_selector",
                "Restricts the revised model's objects instead of the rule's selection.",
                "Schränkt die Objekte des überarbeiteten Modells ein, anstelle der Auswahl der Regel.",
            ),
            param!(
                "match_length_tolerance",
                "Length tolerance in metres of the geometry, placement and related matchers; 0.001 by default.",
                "Längentoleranz in Metern der Zuordnung über Geometrie, Platzierung und Beziehungen; standardmäßig 0,001.",
            ),
            param!(
                "match_angle_tolerance",
                "Angle tolerance in degrees of the placement matcher; 0.01 by default.",
                "Winkeltoleranz in Grad der Zuordnung über die Platzierung; standardmäßig 0,01.",
            ),
            param!(
                "minimum_overlap_ratio",
                "Least overlap share, above 0 and at most 1, for the overlap matcher.",
                "Geringster Überlappungsanteil, über 0 und höchstens 1, für die Zuordnung über Überlappung.",
            ),
            param!(
                "match_path",
                "Relationship steps of the related matcher.",
                "Beziehungsschritte der Zuordnung über verbundene Objekte.",
            ),
            param!(
                "compare_timestamps",
                "Compare the header timestamps; a revised model older than its base is a finding.",
                "Die Zeitstempel der Dateiköpfe vergleichen; ein überarbeitetes Modell, das älter als das Ausgangsmodell ist, ist ein Befund.",
            ),
            param!(
                "compare_relationships",
                "Compare each matched object's related objects per relationship kind.",
                "Die verbundenen Objekte jedes zugeordneten Objekts je Beziehungsart vergleichen.",
            ),
            param!(
                "geometry",
                "`bounds` or `mesh`: how geometry is compared.",
                "`bounds` oder `mesh`: wie die Geometrie verglichen wird.",
            ),
            param!(
                "tolerance_metres",
                "With `mesh`: the largest surface distance in metres that counts as unchanged.",
                "Bei `mesh`: der größte Oberflächenabstand in Metern, der als unverändert gilt.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.name-sequence",
        label: en_de("Name sequence", "Namensfolge"),
        help: en_de(
            "Checks that the members of each anchor, ordered by a numeric property, carry whole-number names counting up from a first number by an increment, such as storeys named 1, 2, 3. A member without an order value leaves the anchor not evaluated unless its placement height may stand in.",
            "Prüft, ob die nach einer Zahleneigenschaft geordneten Mitglieder jedes Bezugsobjekts ganzzahlige Namen tragen, die ab einer Startzahl um eine Schrittweite hochzählen, etwa Geschosse mit den Namen 1, 2, 3. Ein Mitglied ohne Ordnungswert lässt das Bezugsobjekt unbewertet, sofern nicht seine Platzierungshöhe einspringen darf.",
        ),
        parameters: &[
            param!(
                "member_selector",
                "The numbered members, such as storeys.",
                "Die nummerierten Mitglieder, etwa Geschosse.",
            ),
            param!(
                "name",
                "Property holding each member's name.",
                "Eigenschaft, die den Namen jedes Mitglieds enthält.",
            ),
            param!(
                "order",
                "Numeric property ordering the members, such as the elevation.",
                "Zahleneigenschaft, die die Mitglieder ordnet, etwa die Höhenlage.",
            ),
            param!(
                "first",
                "Number the first member must carry; 1 by default.",
                "Zahl, die das erste Mitglied tragen muss; standardmäßig 1.",
            ),
            param!(
                "increment",
                "Step between consecutive numbers; 1 by default.",
                "Schrittweite zwischen aufeinanderfolgenden Zahlen; standardmäßig 1.",
            ),
            param!(
                "order_fallback",
                "`placement_height`: order a member without an order value by the height of its placement.",
                "`placement_height`: ein Mitglied ohne Ordnungswert nach der Höhe seiner Platzierung ordnen.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.numbering-consistency",
        label: en_de("Numbering consistency", "Nummerierungskonsistenz"),
        help: en_de(
            "Reads a number from each object's property through a pattern and checks within each scope that the numbers share a prefix and leave no gaps. A value that is missing or does not match the pattern is not evaluated, and so is a gap an unreadable object could fill.",
            "Liest über ein Muster eine Zahl aus einer Eigenschaft jedes Objekts und prüft innerhalb jedes Bereichs, ob die Zahlen ein gemeinsames Präfix haben und lückenlos sind. Ein fehlender oder nicht zum Muster passender Wert bleibt unbewertet, ebenso eine Lücke, die ein nicht lesbares Objekt füllen könnte.",
        ),
        parameters: &[
            param!(
                "property",
                "Property holding the numbered value, such as a space number.",
                "Eigenschaft mit dem nummerierten Wert, etwa einer Raumnummer.",
            ),
            param!(
                "pattern",
                "XML Schema pattern over the whole value with exactly one group capturing the digits.",
                "XML-Schema-Muster über den gesamten Wert mit genau einer Gruppe, die die Ziffern erfasst.",
            ),
            param!(
                "prefix_length",
                "Number of leading digits that must agree across the scope.",
                "Anzahl führender Ziffern, die im ganzen Bereich übereinstimmen müssen.",
            ),
            param!(
                "gap_free",
                "The distinct numbers, sorted, must step by one.",
                "Die verschiedenen Zahlen müssen, sortiert, in Einerschritten aufeinanderfolgen.",
            ),
            param!(
                "across_sources",
                "One scope for the whole project; per source by default.",
                "Ein Bereich für das gesamte Projekt; standardmäßig je Quelle.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.object-count",
        label: en_de("Object count", "Objektanzahl"),
        help: en_de(
            "Checks that the rule's selection holds between a minimum and a maximum number of objects per source or across the project; with neither bound, at least one. Undecided objects that could change the verdict leave the scope not evaluated.",
            "Prüft, ob die Auswahl der Regel je Quelle oder im gesamten Projekt zwischen einer Mindest- und einer Höchstzahl von Objekten umfasst; ohne Grenzen mindestens eines. Unentschiedene Objekte, die das Ergebnis ändern könnten, lassen den Bereich unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum",
                "Least number of objects.",
                "Kleinste Anzahl an Objekten.",
            ),
            param!(
                "maximum",
                "Greatest number of objects.",
                "Größte Anzahl an Objekten.",
            ),
            param!(
                "across_sources",
                "Count across the whole project; per source by default.",
                "Über das gesamte Projekt zählen; standardmäßig je Quelle.",
            ),
            param!(
                "disciplines",
                "Count only the sources playing one of these disciplines.",
                "Nur die Quellen zählen, die eine dieser Fachdisziplinen vertreten.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.opening-area",
        label: en_de("Opening area balance", "Öffnungsflächenbilanz"),
        help: en_de(
            "Checks that the openings of each selected host, such as a wall, account for the difference between its stated gross and net side areas within a tolerance. An opening that cannot be placed, openings that may overlap or an opening of undecided selection leave the host not evaluated.",
            "Prüft, ob die Öffnungen jedes gewählten Bauteils, etwa einer Wand, die Differenz zwischen seiner angegebenen Brutto- und Nettoseitenfläche innerhalb einer Toleranz erklären. Eine nicht platzierbare Öffnung, möglicherweise überlappende Öffnungen oder eine Öffnung mit unentschiedener Auswahl lassen das Bauteil unbewertet.",
        ),
        parameters: &[
            HOST_OPENING_PATH,
            HOST_OPENING_SELECTOR,
            LENGTH_AXIS,
            HEIGHT_AXIS,
            MINIMUM_OPENING_AREA,
            param!(
                "gross_area",
                "The host's stated gross side area.",
                "Die angegebene Bruttoseitenfläche des Bauteils.",
            ),
            param!(
                "net_area",
                "The host's stated net side area.",
                "Die angegebene Nettoseitenfläche des Bauteils.",
            ),
            param!(
                "area_tolerance",
                "Area by which the openings' sum may differ from gross less net area; 0 by default.",
                "Fläche, um die die Summe der Öffnungen von Brutto- minus Nettofläche abweichen darf; standardmäßig 0.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.opening-spaces",
        label: en_de("Spaces at openings", "Räume an Öffnungen"),
        help: en_de(
            "Checks that each selected door, window or opening relates to the spaces its host wall calls for: two, one on each side, in an internal wall, and one in an external wall. An undeclared exposure, disagreeing hosts or an undecided space that could change the count leave the element not evaluated.",
            "Prüft, ob jede gewählte Tür, jedes Fenster oder jede Öffnung mit den Räumen verbunden ist, die ihre Wand erfordert: zwei, je einer auf jeder Seite, in einer Innenwand und einer in einer Außenwand. Eine nicht angegebene Lage, widersprüchliche Wände oder ein unentschiedener Raum, der die Anzahl ändern könnte, lassen das Element unbewertet.",
        ),
        parameters: &[
            param!(
                "host_path",
                "Relationship steps from the element to its host wall.",
                "Beziehungsschritte vom Element zu seiner Wand.",
            ),
            param!(
                "host_selector",
                "Which reached objects are host walls.",
                "Welche erreichten Objekte Wände sind, in denen das Element sitzt.",
            ),
            param!(
                "external_property",
                "The host's boolean exposure, such as IsExternal.",
                "Die boolesche Außenlage der Wand, etwa IsExternal.",
            ),
            param!(
                "space_path",
                "Relationship steps from the element to its spaces.",
                "Beziehungsschritte vom Element zu seinen Räumen.",
            ),
            param!(
                "space_selector",
                "Which reached objects count as spaces; every object by default.",
                "Welche erreichten Objekte als Räume zählen; standardmäßig jedes Objekt.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.opening-zone",
        label: en_de("Opening zones", "Öffnungszonen"),
        help: en_de(
            "Checks that each selected opening lies within its host's face and the allowed zone: distances from ends, edges, flanges, supports and other openings, allowed zones and dimensioning rows. A distance known only from below, or an undecided support or neighbour that may come too close, leaves the opening not evaluated.",
            "Prüft, ob jede gewählte Öffnung innerhalb der Ansichtsfläche ihres Bauteils und in der zulässigen Zone liegt: Abstände zu Enden, Rändern, Flanschen, Auflagern und anderen Öffnungen, zulässige Zonen und Bemaßungszeilen. Ein nur nach unten bekannter Abstand oder ein unentschiedenes Auflager bzw. eine Nachbaröffnung, die zu nah kommen könnte, lässt die Öffnung unbewertet.",
        ),
        parameters: &[
            param!(
                "host_path",
                "Relationship steps from the opening to its host.",
                "Beziehungsschritte von der Öffnung zu ihrem Bauteil.",
            ),
            param!(
                "host_selector",
                "The hosts checked; every object by default.",
                "Die geprüften Bauteile; standardmäßig jedes Objekt.",
            ),
            LENGTH_AXIS,
            HEIGHT_AXIS,
            param!(
                "end_distance",
                "Length the opening must keep from both ends of the host along `length_axis`.",
                "Länge, die die Öffnung entlang `length_axis` zu beiden Enden des Bauteils einhalten muss.",
            ),
            param!(
                "edge_distance",
                "Length the opening must keep from both edges along `height_axis`, or from both flanges with `zone` `web`.",
                "Länge, die die Öffnung entlang `height_axis` zu beiden Rändern einhalten muss, bei `zone` `web` zu beiden Flanschen.",
            ),
            param!(
                "edge_distance_maximum",
                "Largest distance allowed from the edges `maximum_edges` names.",
                "Größter zulässiger Abstand zu den von `maximum_edges` genannten Rändern.",
            ),
            param!(
                "maximum_edges",
                "`top`, `bottom` or `both` (default): the edges `edge_distance_maximum` applies to.",
                "`top`, `bottom` oder `both` (Standard): die Ränder, für die `edge_distance_maximum` gilt.",
            ),
            param!(
                "zone",
                "`section` (default, the host's whole height) or `web` (between the flanges).",
                "`section` (Standard, die gesamte Bauteilhöhe) oder `web` (zwischen den Flanschen).",
            ),
            param!(
                "opening_spacing",
                "Clear distance the opening must keep from every other opening of the host.",
                "Lichter Abstand, den die Öffnung zu jeder anderen Öffnung des Bauteils einhalten muss.",
            ),
            param!(
                "zones",
                "Rows of allowed zones, each the face less insets from the ends, bottom and top.",
                "Zeilen zulässiger Zonen, jeweils die Ansichtsfläche abzüglich Randabständen an Enden, Unter- und Oberseite.",
            ),
            MINIMUM_OPENING_AREA,
            param!(
                "dimensions",
                "Rows bounding distances from an opening to the nearest other opening or to an edge of the host.",
                "Zeilen, die Abstände einer Öffnung zur nächsten anderen Öffnung oder zu einem Rand des Bauteils begrenzen.",
            ),
            param!(
                "support_path",
                "Relationship steps from the host to its supports and connecting members.",
                "Beziehungsschritte vom Bauteil zu seinen Auflagern und anschließenden Bauteilen.",
            ),
            param!(
                "support_selector",
                "Objects that may be supports; every object by default.",
                "Objekte, die Auflager sein können; standardmäßig jedes Objekt.",
            ),
            param!(
                "support_gap",
                "Objects this close to the host in space are its supports too.",
                "Objekte, die räumlich so nah am Bauteil liegen, sind ebenfalls seine Auflager.",
            ),
            param!(
                "support_distance",
                "Length the opening must keep from each support along the length; with a ratio, the minimum.",
                "Länge, die die Öffnung entlang der Länge zu jedem Auflager einhalten muss; mit einem Verhältnis das Minimum.",
            ),
            param!(
                "support_distance_ratio",
                "Fraction of the host's span or depth the opening must keep from each support.",
                "Anteil der Spannweite oder Höhe des Bauteils, den die Öffnung zu jedem Auflager einhalten muss.",
            ),
            param!(
                "support_distance_reference",
                "`span` or `depth`: what `support_distance_ratio` is a fraction of.",
                "`span` oder `depth`: worauf sich `support_distance_ratio` bezieht.",
            ),
            param!(
                "support_clearance",
                "Clear distance the opening must keep from each support's footprint in the face.",
                "Lichter Abstand, den die Öffnung zur Fläche jedes Auflagers in der Ansicht einhalten muss.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.parking-bay",
        label: en_de("Parking bay", "Stellplatz"),
        help: en_de(
            "Checks each selected parking bay along its own axes: width, length and height, its orientation to an aisle and its obstructed ends and sides, or uses orientation and obstructions to select the bays the size bounds apply to. A footprint without a unique orientation, or a measure straddling a bound, is not evaluated.",
            "Prüft jeden gewählten Stellplatz entlang seiner eigenen Achsen: Breite, Länge und Höhe, seine Ausrichtung zu einer Fahrgasse und seine verstellten Enden und Seiten, oder wählt über Ausrichtung und Hindernisse die Stellplätze aus, für die die Größengrenzen gelten. Ein Grundriss ohne eindeutige Ausrichtung oder ein Maß, das eine Grenze überspannt, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "min_width",
                "Least width of the bay, inclusive.",
                "Geringste Breite des Stellplatzes, einschließlich.",
            ),
            param!(
                "max_width",
                "Greatest width of the bay, inclusive.",
                "Größte Breite des Stellplatzes, einschließlich.",
            ),
            param!(
                "min_length",
                "Least length of the bay, inclusive.",
                "Geringste Länge des Stellplatzes, einschließlich.",
            ),
            param!(
                "max_length",
                "Greatest length of the bay, inclusive.",
                "Größte Länge des Stellplatzes, einschließlich.",
            ),
            param!(
                "min_height",
                "Least height of the bay's vertical extent, inclusive.",
                "Geringste lichte Höhe des Stellplatzes, einschließlich.",
            ),
            param!(
                "max_height",
                "Greatest height of the bay's vertical extent, inclusive.",
                "Größte lichte Höhe des Stellplatzes, einschließlich.",
            ),
            param!(
                "aisles",
                "The aisles a bay opens onto.",
                "Die Fahrgassen, an die ein Stellplatz anschließt.",
            ),
            param!(
                "aisle_reach",
                "How far in plan an aisle may lie from the bay; 0 by default, meeting it.",
                "Wie weit eine Fahrgasse im Grundriss vom Stellplatz entfernt liegen darf; standardmäßig 0, also angrenzend.",
            ),
            param!(
                "orientation",
                "`parallel`, `perpendicular` or `angled`: how the bay's long axis must stand to an aisle's.",
                "`parallel`, `perpendicular` oder `angled`: wie die Längsachse des Stellplatzes zu der einer Fahrgasse stehen muss.",
            ),
            param!(
                "angle_tolerance",
                "How far from parallel or perpendicular still counts as such; in [0°, 45°).",
                "Wie weit von parallel oder senkrecht noch als solches gilt; in [0°, 45°).",
            ),
            param!(
                "obstacles",
                "What obstructs a bay, such as columns and walls.",
                "Was einen Stellplatz verstellt, etwa Stützen und Wände.",
            ),
            param!(
                "obstruction_reach",
                "How far in plan from the bay an obstacle obstructs it.",
                "Bis zu welchem Abstand im Grundriss ein Hindernis den Stellplatz verstellt.",
            ),
            param!(
                "end_obstructions",
                "`none`, `one` or `both`: how many ends may be obstructed.",
                "`none`, `one` oder `both`: wie viele Enden verstellt sein dürfen.",
            ),
            param!(
                "side_obstructions",
                "`none`, `one` or `both`: how many sides may be obstructed.",
                "`none`, `one` oder `both`: wie viele Seiten verstellt sein dürfen.",
            ),
            param!(
                "applies_when",
                "`findings` (default): orientation and obstructions are findings; `filter`: they select the bays the size bounds apply to.",
                "`findings` (Standard): Ausrichtung und Hindernisse sind Befunde; `filter`: sie wählen die Stellplätze, für die die Größengrenzen gelten.",
            ),
            param!(
                "orientations",
                "With `filter`: allowed orientation states, any of `parallel`, `perpendicular`, `angled` and `unclear`.",
                "Bei `filter`: zulässige Ausrichtungen, beliebige aus `parallel`, `perpendicular`, `angled` und `unclear`.",
            ),
            param!(
                "end_states",
                "With `filter`: allowed numbers of obstructed ends, any of `none`, `one` and `both`.",
                "Bei `filter`: zulässige Anzahlen verstellter Enden, beliebige aus `none`, `one` und `both`.",
            ),
            param!(
                "side_states",
                "With `filter`: allowed numbers of obstructed sides, any of `none`, `one` and `both`.",
                "Bei `filter`: zulässige Anzahlen verstellter Seiten, beliebige aus `none`, `one` und `both`.",
            ),
            param!(
                "side_zone_length",
                "A side counts as obstructed only by an obstacle overlapping its central stretch of this length.",
                "Eine Seite gilt nur durch ein Hindernis als verstellt, das ihren mittleren Abschnitt dieser Länge überlappt.",
            ),
            param!(
                "neighbour_reach",
                "Infer the orientation from neighbouring bays within this reach instead of from aisles.",
                "Die Ausrichtung aus benachbarten Stellplätzen innerhalb dieses Abstands statt aus Fahrgassen ableiten.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.plan-area",
        label: en_de("Plan area", "Grundrissfläche"),
        help: en_de(
            "Checks that each selected object's footprint or facade area, or the summed areas of its members, lies within the bounds in square metres. An area straddling a bound, an empty footprint or an undecided member that could change the sum is not evaluated.",
            "Prüft, ob die Grundriss- oder Fassadenfläche jedes gewählten Objekts oder die Summe der Flächen seiner Mitglieder innerhalb der Grenzen in Quadratmetern liegt. Eine Fläche, die eine Grenze überspannt, ein leerer Grundriss oder ein unentschiedenes Mitglied, das die Summe ändern könnte, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum",
                "Least area in square metres, inclusive.",
                "Kleinste Fläche in Quadratmetern, einschließlich.",
            ),
            param!(
                "maximum",
                "Greatest area in square metres, inclusive.",
                "Größte Fläche in Quadratmetern, einschließlich.",
            ),
            param!(
                "member_selector",
                "Members whose areas are summed instead of the object's own.",
                "Mitglieder, deren Flächen anstelle der eigenen Fläche des Objekts summiert werden.",
            ),
            param!(
                "measure",
                "`footprint` (default) or `facade`: what is measured.",
                "`footprint` (Standard) oder `facade`: was gemessen wird.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.plan-coverage",
        label: en_de("Plan coverage", "Grundrissüberdeckung"),
        help: en_de(
            "Checks that each subject's footprint lies within one candidate object, such as a fire compartment, by at least a minimum share. A share that may reach the minimum but is not certain to leaves the subject not evaluated.",
            "Prüft, ob der Grundriss jedes gewählten Objekts mindestens zu einem Mindestanteil innerhalb eines Kandidatenobjekts liegt, etwa eines Brandabschnitts. Ein Anteil, der das Minimum erreichen kann, aber nicht sicher erreicht, lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "candidate_selector",
                "The objects a subject must lie within.",
                "Die Objekte, innerhalb derer ein gewähltes Objekt liegen muss.",
            ),
            param!(
                "minimum_ratio",
                "Least share of the subject's footprint a candidate must cover.",
                "Geringster Anteil des Grundrisses, den ein Kandidat überdecken muss.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-comparison",
        label: en_de("Property comparison", "Eigenschaftsvergleich"),
        help: en_de(
            "Compares a property of candidate objects, the object itself, its group, related objects or objects sharing a container, with a target, or compares their count or sum. Incompatible types or unavailable evidence leave the object not evaluated; a missing property is a finding.",
            "Vergleicht eine Eigenschaft von Kandidatenobjekten, dem Objekt selbst, seiner Gruppe, verbundenen Objekten oder Objekten im selben Container, mit einem Zielwert, oder vergleicht deren Anzahl oder Summe. Unvereinbare Typen oder fehlende Nachweise lassen das Objekt unbewertet; eine fehlende Eigenschaft ist ein Befund.",
        ),
        parameters: &[
            param!(
                "compared_selector",
                "Picks the candidate objects whose property is compared.",
                "Wählt die Kandidatenobjekte, deren Eigenschaft verglichen wird.",
            ),
            param!(
                "compared_property",
                "The candidates' property compared; not needed to compare a count.",
                "Die verglichene Eigenschaft der Kandidaten; für einen Anzahlvergleich nicht erforderlich.",
            ),
            param!(
                "target_property",
                "Target: a property of the checked object.",
                "Zielwert: eine Eigenschaft des geprüften Objekts.",
            ),
            param!(
                "target_number",
                "Target: a constant number.",
                "Zielwert: eine konstante Zahl.",
            ),
            param!(
                "target_quantity",
                "Target: a constant quantity with its unit.",
                "Zielwert: eine konstante Größe mit Einheit.",
            ),
            param!(
                "target_text",
                "Target: a constant text.",
                "Zielwert: ein konstanter Text.",
            ),
            param!(
                "target_texts",
                "Target: texts or patterns for the text operators; any entry satisfies.",
                "Zielwert: Texte oder Muster für die Textoperatoren; jeder Eintrag genügt.",
            ),
            param!(
                "target_boolean",
                "Target: a constant truth value.",
                "Zielwert: ein konstanter Wahrheitswert.",
            ),
            param!(
                "target_date",
                "Target: a constant date.",
                "Zielwert: ein konstantes Datum.",
            ),
            param!(
                "target_date_time",
                "Target: a constant date-time.",
                "Zielwert: ein konstanter Zeitpunkt.",
            ),
            param!(
                "precision",
                "`day`: compare date-times with dates by the day they state.",
                "`day`: Zeitpunkte mit Daten nach dem angegebenen Tag vergleichen.",
            ),
            param!(
                "minimum_number",
                "With `between`: lower bound, a number.",
                "Bei `between`: Untergrenze, eine Zahl.",
            ),
            param!(
                "maximum_number",
                "With `between`: upper bound, a number.",
                "Bei `between`: Obergrenze, eine Zahl.",
            ),
            param!(
                "minimum_quantity",
                "With `between`: lower bound, a quantity.",
                "Bei `between`: Untergrenze, eine Größe.",
            ),
            param!(
                "maximum_quantity",
                "With `between`: upper bound, a quantity.",
                "Bei `between`: Obergrenze, eine Größe.",
            ),
            param!(
                "minimum_date",
                "With `between`: lower bound, a date.",
                "Bei `between`: Untergrenze, ein Datum.",
            ),
            param!(
                "maximum_date",
                "With `between`: upper bound, a date.",
                "Bei `between`: Obergrenze, ein Datum.",
            ),
            param!(
                "minimum_date_time",
                "With `between`: lower bound, a date-time.",
                "Bei `between`: Untergrenze, ein Zeitpunkt.",
            ),
            param!(
                "maximum_date_time",
                "With `between`: upper bound, a date-time.",
                "Bei `between`: Obergrenze, ein Zeitpunkt.",
            ),
            param!(
                "minimum_property",
                "With `between`: lower bound read from a property of the checked object.",
                "Bei `between`: Untergrenze aus einer Eigenschaft des geprüften Objekts.",
            ),
            param!(
                "maximum_property",
                "With `between`: upper bound read from a property of the checked object.",
                "Bei `between`: Obergrenze aus einer Eigenschaft des geprüften Objekts.",
            ),
            param!(
                "operator",
                "The comparison, such as `equals`, `greater`, `between`, `like`, `one_of` or `is_defined`.",
                "Der Vergleich, etwa `equals`, `greater`, `between`, `like`, `one_of` oder `is_defined`.",
            ),
            param!(
                "case_sensitive",
                "Whether text comparisons respect case; true by default.",
                "Ob Textvergleiche Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "factor",
                "Factor the compared value is multiplied by before comparing; 1 by default.",
                "Faktor, mit dem der verglichene Wert vor dem Vergleich multipliziert wird; standardmäßig 1.",
            ),
            param!(
                "component_mode",
                "`checked`, `shared`, `related`, `same_space` or `same_building`: which objects are candidates.",
                "`checked`, `shared`, `related`, `same_space` oder `same_building`: welche Objekte Kandidaten sind.",
            ),
            param!(
                "container_selector",
                "With `same_space` and `same_building`: the containers objects share, such as spaces or buildings.",
                "Bei `same_space` und `same_building`: die gemeinsamen Container, etwa Räume oder Gebäude.",
            ),
            CONTAINER_RELATIONSHIP,
            LEVEL_PROPERTY,
            param!(
                "quantifier",
                "`count` or `sum`: compare the candidates' number or total with the target.",
                "`count` oder `sum`: Anzahl oder Summe der Kandidaten mit dem Zielwert vergleichen.",
            ),
            CATEGORY_PROPERTY,
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
            TOLERANCE,
            RELATIVE_TOLERANCE,
            DECIMALS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-data-type",
        label: en_de("Property data type", "Datentyp einer Eigenschaft"),
        help: en_de(
            "Requires a non-empty property whose type, as the source declares it, equals the given data type. A present value whose type the source does not report is not evaluated.",
            "Verlangt eine nicht leere Eigenschaft, deren von der Quelle angegebener Typ dem vorgegebenen Datentyp entspricht. Ein vorhandener Wert, dessen Typ die Quelle nicht angibt, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "property",
                "The property checked.",
                "Die geprüfte Eigenschaft.",
            ),
            param!(
                "data_type",
                "The required declared type, such as IFCLABEL, compared ignoring case.",
                "Der geforderte angegebene Typ, etwa IFCLABEL, ohne Beachtung der Groß- und Kleinschreibung verglichen.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-exists",
        label: en_de("Property exists", "Eigenschaft vorhanden"),
        help: en_de(
            "Requires the property to be present on each selected object, even with a null or blank value. Absence counts only on exact evidence; an incomplete answer leaves the object not evaluated.",
            "Verlangt, dass die Eigenschaft an jedem gewählten Objekt vorhanden ist, auch mit leerem Wert. Ein Fehlen zählt nur bei exaktem Nachweis; eine unvollständige Antwort lässt das Objekt unbewertet.",
        ),
        parameters: &[param!(
            "property",
            "The property that must be present.",
            "Die Eigenschaft, die vorhanden sein muss.",
        )],
    },
    CapabilityText {
        id: "axioval:capability.property-predicate",
        label: en_de("Property predicate", "Eigenschaftsbedingung"),
        help: en_de(
            "Compares one property of each selected object with exactly one target under an operator: a number, quantity, text, list of texts, truth value or date. A quantity compared with a unitless target, or a value of another type, is not evaluated.",
            "Vergleicht eine Eigenschaft jedes gewählten Objekts mit genau einem Zielwert über einen Operator: eine Zahl, Größe, ein Text, eine Textliste, ein Wahrheitswert oder ein Datum. Eine mit einem einheitenlosen Zielwert verglichene Größe oder ein Wert anderen Typs bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "property_set",
                "Name of the property set.",
                "Name des Eigenschaftssatzes.",
            ),
            param!("property", "Name of the property.", "Name der Eigenschaft."),
            param!(
                "operator",
                "The comparison, such as `equal`, `less_than`, `matches`, `one_of` or `is_defined`.",
                "Der Vergleich, etwa `equal`, `less_than`, `matches`, `one_of` oder `is_defined`.",
            ),
            param!(
                "value",
                "Target: an integer; may be computed per object.",
                "Zielwert: eine ganze Zahl; kann je Objekt berechnet werden.",
            ),
            param!(
                "number",
                "Target: a decimal number; may be computed per object.",
                "Zielwert: eine Dezimalzahl; kann je Objekt berechnet werden.",
            ),
            param!(
                "quantity",
                "Target: a quantity with its unit, compared in SI units.",
                "Zielwert: eine Größe mit Einheit, verglichen in SI-Einheiten.",
            ),
            param!("text", "Target: a text.", "Zielwert: ein Text."),
            param!(
                "texts",
                "Target: a list of texts for `one_of` and `none_of`.",
                "Zielwert: eine Textliste für `one_of` und `none_of`.",
            ),
            param!(
                "boolean",
                "Target: a truth value.",
                "Zielwert: ein Wahrheitswert.",
            ),
            param!("date", "Target: a date.", "Zielwert: ein Datum."),
            param!(
                "date_time",
                "Target: a date-time.",
                "Zielwert: ein Zeitpunkt.",
            ),
            param!(
                "precision",
                "`day`: compare date-times and dates by the day they state.",
                "`day`: Zeitpunkte und Daten nach dem angegebenen Tag vergleichen.",
            ),
            param!(
                "case_sensitive",
                "Whether text comparisons respect case; true by default.",
                "Ob Textvergleiche Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            TOLERANCE,
            RELATIVE_TOLERANCE,
            DECIMALS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-required",
        label: en_de("Required property value", "Pflichtwert einer Eigenschaft"),
        help: en_de(
            "Requires each selected object to carry a non-empty value of the property; absence, null and blank text are findings. A failure to resolve the property leaves the object not evaluated.",
            "Verlangt, dass jedes gewählte Objekt einen nicht leeren Wert der Eigenschaft trägt; Fehlen, Nullwert und leerer Text sind Befunde. Kann die Eigenschaft nicht aufgelöst werden, bleibt das Objekt unbewertet.",
        ),
        parameters: &[param!(
            "property",
            "The property that must hold a value.",
            "Die Eigenschaft, die einen Wert haben muss.",
        )],
    },
    CapabilityText {
        id: "axioval:capability.property-requirements",
        label: en_de("Property requirements", "Eigenschaftsanforderungen"),
        help: en_de(
            "Checks each selected object against a requirements table stating which properties it must, may or must not carry and which values they may hold. An undecided row, a value of another kind or a refused enumeration leaves the object not evaluated.",
            "Prüft jedes gewählte Objekt gegen eine Anforderungstabelle, die festlegt, welche Eigenschaften es tragen muss, darf oder nicht darf und welche Werte sie haben dürfen. Eine unentschiedene Zeile, ein Wert anderer Art oder eine verweigerte Aufzählung lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "requirements",
                "Requirement rows: applicability, property set and property by name or pattern, requirement or state, and value conditions.",
                "Anforderungszeilen: Geltungsbereich, Eigenschaftssatz und Eigenschaft als Name oder Muster, Anforderung oder Zustand sowie Wertbedingungen.",
            ),
            param!(
                "case_sensitive",
                "Whether `value_like` and `one_of` respect case; true by default.",
                "Ob `value_like` und `one_of` Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "area_property",
                "Area property the value is divided by for `per` `stated-area`.",
                "Flächeneigenschaft, durch die der Wert bei `per` `stated-area` geteilt wird.",
            ),
            param!(
                "volume_property",
                "Volume property the value is divided by for `per` `stated-volume`.",
                "Volumeneigenschaft, durch die der Wert bei `per` `stated-volume` geteilt wird.",
            ),
            param!(
                "group_by_value",
                "Merge the findings of one row with the same result and value into one finding.",
                "Die Befunde einer Zeile mit gleichem Ergebnis und Wert zu einem Befund zusammenfassen.",
            ),
            CATEGORY_PROPERTY,
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-value",
        label: en_de(
            "Property value constraints",
            "Wertbeschränkungen einer Eigenschaft",
        ),
        help: en_de(
            "Checks a property's value against constraints cast to its kind: enumerated values, patterns, bounds, lengths, digits and data type, with property names optionally given as patterns. A literal that cannot be cast, or a quantity without `si_units`, leaves the object not evaluated.",
            "Prüft den Wert einer Eigenschaft gegen Einschränkungen, die in ihre Art umgewandelt werden: aufgezählte Werte, Muster, Grenzen, Längen, Stellenzahlen und Datentyp, wobei Eigenschaftsnamen auch als Muster angegeben werden können. Ein nicht umwandelbares Literal oder eine Größe ohne `si_units` lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "property",
                "The property checked, or none when properties are named by pattern.",
                "Die geprüfte Eigenschaft, oder keine, wenn Eigenschaften über Muster benannt werden.",
            ),
            param!(
                "property_set_pattern",
                "XML Schema pattern over property set names, with `property_pattern`.",
                "XML-Schema-Muster über Namen von Eigenschaftssätzen, zusammen mit `property_pattern`.",
            ),
            param!(
                "property_pattern",
                "XML Schema pattern over property names, instead of `property`.",
                "XML-Schema-Muster über Eigenschaftsnamen, anstelle von `property`.",
            ),
            param!(
                "data_type",
                "The required declared data type.",
                "Der geforderte angegebene Datentyp.",
            ),
            param!(
                "min_inclusive",
                "Inclusive lower bound, written as a literal.",
                "Untergrenze einschließlich, als Literal angegeben.",
            ),
            param!(
                "max_inclusive",
                "Inclusive upper bound, written as a literal.",
                "Obergrenze einschließlich, als Literal angegeben.",
            ),
            param!(
                "min_exclusive",
                "Exclusive lower bound, written as a literal.",
                "Untergrenze ausschließlich, als Literal angegeben.",
            ),
            param!(
                "max_exclusive",
                "Exclusive upper bound, written as a literal.",
                "Obergrenze ausschließlich, als Literal angegeben.",
            ),
            param!(
                "precision",
                "`day`: date and date-time literals compare by the day they state.",
                "`day`: Datums- und Zeitpunktliterale werden nach dem angegebenen Tag verglichen.",
            ),
            param!(
                "quantifier",
                "`any` or `all`: how a list, a bounded value or a table is judged.",
                "`any` oder `all`: wie eine Liste, ein Wertebereich oder eine Tabelle bewertet wird.",
            ),
            param!(
                "values",
                "Allowed values; the value must equal one of them.",
                "Zulässige Werte; der Wert muss einem davon entsprechen.",
            ),
            param!(
                "patterns",
                "XML Schema patterns, one of which the whole text must match.",
                "XML-Schema-Muster, von denen der gesamte Text einem entsprechen muss.",
            ),
            param!(
                "length",
                "Exact length of the text.",
                "Genaue Länge des Texts.",
            ),
            param!(
                "min_length",
                "Least length of the text.",
                "Geringste Länge des Texts.",
            ),
            param!(
                "max_length",
                "Greatest length of the text.",
                "Größte Länge des Texts.",
            ),
            param!(
                "total_digits",
                "Most digits a number may have in total.",
                "Höchstzahl an Ziffern, die eine Zahl insgesamt haben darf.",
            ),
            param!(
                "fraction_digits",
                "Most fraction digits a number may have.",
                "Höchstzahl an Nachkommastellen, die eine Zahl haben darf.",
            ),
            param!(
                "optional",
                "An absent or null property passes; a present value is checked.",
                "Eine fehlende oder leere Eigenschaft besteht; ein vorhandener Wert wird geprüft.",
            ),
            param!(
                "si_units",
                "Read numeric literals in the quantity's coherent SI unit, so that quantities are compared.",
                "Zahlenliterale in der kohärenten SI-Einheit der Größe lesen, damit Größen verglichen werden.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.property-value-equals",
        label: en_de("Boolean property value", "Wahrheitswert einer Eigenschaft"),
        help: en_de(
            "Requires a boolean property of each selected object to equal the expected value. A value that is not a truth value is invalid evidence and leaves the object not evaluated.",
            "Verlangt, dass eine boolesche Eigenschaft jedes gewählten Objekts dem erwarteten Wert entspricht. Ein Wert, der kein Wahrheitswert ist, gilt als ungültiger Nachweis und lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "property",
                "The boolean property checked.",
                "Die geprüfte boolesche Eigenschaft.",
            ),
            param!(
                "expected",
                "The required truth value.",
                "Der geforderte Wahrheitswert.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.quantity-takeoff",
        label: en_de("Quantity takeoff", "Mengenermittlung"),
        help: en_de(
            "Counts the selected objects per group and aggregates stated, measured, related and computed values into a report table; it raises no finding. Objects whose group or selection is undecided widen the figures and are reported not evaluated.",
            "Zählt die gewählten Objekte je Gruppe und fasst angegebene, gemessene, verbundene und berechnete Werte in einer Berichtstabelle zusammen; Befunde entstehen keine. Objekte mit unentschiedener Gruppe oder Auswahl erweitern die Werte und werden als unbewertet gemeldet.",
        ),
        parameters: takeoff_parameters!(groups: 1 2 3; measures: 1 2 3 4 5 6 7 8),
    },
    CapabilityText {
        id: "axioval:capability.ramp-geometry",
        label: en_de("Ramp geometry", "Rampengeometrie"),
        help: en_de(
            "Measures each selected ramp's sloped runs and checks slope, width, landings, headroom, handrails, end spaces and clear width. A measurement straddling its bound, or a ramp the service cannot measure, is not evaluated.",
            "Misst die geneigten Läufe jeder gewählten Rampe und prüft Neigung, Breite, Podeste, Kopfhöhe, Handläufe, Bewegungsflächen an den Enden und lichte Breite. Ein Messwert, der seine Grenze überspannt, oder eine vom Dienst nicht messbare Rampe bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "slope_limits",
                "Rows of maximum slope as rise over length, optionally with a maximum run length and rise; a run conforms when some row holds.",
                "Zeilen mit maximaler Neigung als Höhe zu Länge, optional mit maximaler Lauflänge und Höhe; ein Lauf entspricht, wenn eine Zeile erfüllt ist.",
            ),
            param!(
                "slope_tolerance",
                "Largest difference between the ramp's steepest and shallowest run.",
                "Größter Unterschied zwischen dem steilsten und dem flachsten Lauf der Rampe.",
            ),
            MINIMUM_HEADROOM,
            HEADROOM_OBSTACLES,
            WIDTH_MINIMUM,
            WIDTH_MAXIMUM,
            LANDING_OBJECTS,
            LANDING_DEPTH_MINIMUM,
            LANDING_WIDTH_MINIMUM,
            LANDING_AT_LEAST_WALKING_WIDTH,
            LANDINGS_REQUIRED,
            MINIMUM_HEADROOM_BELOW,
            HEADROOM_BELOW_SPACES,
            HANDRAIL_OBJECTS,
            HANDRAIL_REACH_ACROSS,
            HANDRAIL_REACH_ABOVE,
            HANDRAIL_HEIGHT_MINIMUM,
            HANDRAIL_HEIGHT_MAXIMUM,
            HANDRAIL_EXTENSION_MINIMUM,
            HANDRAIL_EXTENSION_MAXIMUM,
            HANDRAIL_GAP_MAXIMUM,
            HANDRAIL_SIDES,
            HANDRAIL_BOTH_SIDES_ABOVE_WIDTH,
            LANDING_DOORS,
            LANDING_DOOR_HEIGHT,
            LANDING_DOOR_SWING,
            END_SPACE_DEPTH,
            END_SPACE_WIDTH,
            END_SPACE_HEIGHT,
            END_SPACE_OBSTACLES,
            CLEAR_WIDTH_MINIMUM,
            CLEAR_WIDTH_OBSTACLES,
            CLEAR_WIDTH_BAND_FROM,
            CLEAR_WIDTH_BAND_TO,
            param!(
                "end_landing_depth_minimum",
                "Least depth of the landings at the ramp's two ends, instead of `landing_depth_minimum`; may be computed per object.",
                "Geringste Tiefe der Podeste an beiden Enden der Rampe, anstelle von `landing_depth_minimum`; kann je Objekt berechnet werden.",
            ),
            param!(
                "end_landing_width_minimum",
                "Least width of the landings at the ramp's two ends, instead of `landing_width_minimum`; may be computed per object.",
                "Geringste Breite der Podeste an beiden Enden der Rampe, anstelle von `landing_width_minimum`; kann je Objekt berechnet werden.",
            ),
            param!(
                "check_continuous_handrails",
                "The handrail along each side must continue across every landing between runs.",
                "Der Handlauf jeder Seite muss über jedes Zwischenpodest hinweg durchlaufen.",
            ),
            param!(
                "handrail_continuity_tolerance",
                "Largest gap along a side across such a landing; `handrail_gap_maximum` without it.",
                "Größte Lücke an einer Seite über ein solches Podest hinweg; ohne Angabe `handrail_gap_maximum`.",
            ),
            param!(
                "check_rails_obstruction",
                "No rail of the ramp may reach over an `accessible_surface_selector` surface in plan.",
                "Kein Handlauf der Rampe darf im Grundriss über eine Fläche aus `accessible_surface_selector` ragen.",
            ),
            param!(
                "accessible_surface_selector",
                "The clear paths rails must keep out of, such as the zones of adjoining landings or corridors.",
                "Die freizuhaltenden Wege, in die Handläufe nicht ragen dürfen, etwa Zonen angrenzender Podeste oder Flure.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.recess-width",
        label: en_de("Recess width", "Breite von Rücksprüngen"),
        help: en_de(
            "Checks that every recess of each selected object's footprint, a pocket between its outline and its convex hull, is wide enough for its depth. A depth or width straddling a bound, or a tessellated footprint, is not evaluated.",
            "Prüft, ob jeder Rücksprung im Grundriss jedes gewählten Objekts, eine Einbuchtung zwischen Umriss und konvexer Hülle, für seine Tiefe breit genug ist. Eine Tiefe oder Breite, die eine Grenze überspannt, oder ein tesselierter Grundriss bleibt unbewertet.",
        ),
        parameters: &[param!(
            "requirements",
            "Rows by depth range with the required width or width per metre of depth; the first row holding a recess's depth applies.",
            "Zeilen nach Tiefenbereich mit geforderter Breite oder Breite je Meter Tiefe; es gilt die erste Zeile, die die Tiefe des Rücksprungs umfasst.",
        )],
    },
    CapabilityText {
        id: "axioval:capability.related-count",
        label: en_de("Related count", "Anzahl verbundener Objekte"),
        help: en_de(
            "Checks that each anchor has between a minimum and a maximum of related objects a selector picks, reached through a relationship or anywhere in its source. Undecided related objects that could change the verdict leave the anchor not evaluated.",
            "Prüft, ob jedes Bezugsobjekt zwischen einer Mindest- und einer Höchstzahl verbundener, von einem Selektor gewählter Objekte hat, erreicht über eine Beziehung oder überall in seiner Quelle. Unentschiedene verbundene Objekte, die das Ergebnis ändern könnten, lassen das Bezugsobjekt unbewertet.",
        ),
        parameters: &[
            param!(
                "related_selector",
                "Which related objects are counted.",
                "Welche verbundenen Objekte gezählt werden.",
            ),
            param!(
                "minimum",
                "Least number of related objects.",
                "Kleinste Anzahl verbundener Objekte.",
            ),
            param!(
                "maximum",
                "Greatest number of related objects.",
                "Größte Anzahl verbundener Objekte.",
            ),
            param!(
                "same_ends",
                "A path; a related object counts only when it reaches the same objects along it as the anchor.",
                "Ein Pfad; ein verbundenes Objekt zählt nur, wenn es darüber dieselben Objekte erreicht wie das Bezugsobjekt.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.relative-count",
        label: en_de("Relative count", "Anzahlverhältnis"),
        help: en_de(
            "Checks at each anchor, or in each group of a property's value, that the provided objects stand in the required relation to the required ones, by a ratio, a small-count rule or a table. An undecided member or an unreadable group value leaves the anchor or group not evaluated.",
            "Prüft für jedes Bezugsobjekt oder jede Gruppe gleichen Eigenschaftswerts, ob die vorhandenen Objekte im geforderten Verhältnis zu den erfordernden stehen, über ein Verhältnis, eine Regel für kleine Anzahlen oder eine Tabelle. Ein unentschiedenes Mitglied oder ein nicht lesbarer Gruppenwert lässt Bezugsobjekt oder Gruppe unbewertet.",
        ),
        parameters: &[
            param!(
                "provided_selector",
                "The provided objects counted, such as washbasins.",
                "Die gezählten vorhandenen Objekte, etwa Waschtische.",
            ),
            param!(
                "required_selector",
                "The objects that require them, such as workplaces.",
                "Die Objekte, die sie erfordern, etwa Arbeitsplätze.",
            ),
            param!(
                "provided_unit",
                "Ratio mode: the provided count is divided by this before comparing.",
                "Verhältnismodus: die vorhandene Anzahl wird vor dem Vergleich durch diesen Wert geteilt.",
            ),
            param!(
                "required_unit",
                "Ratio mode: the required count is divided by this before comparing.",
                "Verhältnismodus: die erforderliche Anzahl wird vor dem Vergleich durch diesen Wert geteilt.",
            ),
            param!(
                "operator",
                "`equal`, `not_equal`, `greater`, `at_least`, `less` or `at_most`.",
                "`equal`, `not_equal`, `greater`, `at_least`, `less` oder `at_most`.",
            ),
            param!(
                "small_required_below",
                "Required counts from 1 up to below this are judged against `small_provided` instead.",
                "Erforderliche Anzahlen von 1 bis unter diesen Wert werden stattdessen gegen `small_provided` bewertet.",
            ),
            param!(
                "small_provided",
                "The provided count compared for small required counts.",
                "Die vorhandene Anzahl, die bei kleinen erforderlichen Anzahlen verglichen wird.",
            ),
            param!(
                "table",
                "Rows written `R:P`: from R required objects on, at least P provided.",
                "Zeilen in der Form `R:P`: ab R erfordernden Objekten mindestens P vorhandene.",
            ),
            param!(
                "additional_required",
                "Beyond the last table row, each further this many required objects need `additional_provided` more.",
                "Über die letzte Tabellenzeile hinaus erfordern je so viele weitere Objekte `additional_provided` zusätzliche.",
            ),
            param!(
                "additional_provided",
                "How many more provided objects each `additional_required` step needs.",
                "Wie viele zusätzliche vorhandene Objekte jeder Schritt von `additional_required` erfordert.",
            ),
            param!(
                "group_property",
                "Count in groups of this property's value instead of per anchor.",
                "In Gruppen gleichen Werts dieser Eigenschaft zählen statt je Bezugsobjekt.",
            ),
            param!(
                "across_sources",
                "With `group_property`, groups span every source; per source by default.",
                "Bei `group_property` umfassen Gruppen alle Quellen; standardmäßig je Quelle.",
            ),
            param!(
                "case_sensitive",
                "With `group_property`, whether group values respect case; false by default.",
                "Bei `group_property`, ob Gruppenwerte Groß- und Kleinschreibung beachten; standardmäßig false.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.same-container",
        label: en_de("Same container", "Gleicher Container"),
        help: en_de(
            "Checks that each selected object lies in the same nearest containers, such as storeys, as every counterpart reached from it, for example a door and its host wall. An undecided container, counterpart or relationship answer leaves the object not evaluated unless a decided counterpart already differs.",
            "Prüft, ob jedes gewählte Objekt in denselben nächsten Containern, etwa Geschossen, liegt wie jedes von ihm erreichte Gegenobjekt, zum Beispiel eine Tür und ihre Wand. Ein unentschiedener Container, ein unentschiedenes Gegenobjekt oder eine unentschiedene Beziehungsantwort lässt das Objekt unbewertet, sofern nicht bereits ein entschiedenes Gegenobjekt abweicht.",
        ),
        parameters: &[
            param!(
                "counterpart_path",
                "Relationship steps from the object to its counterparts.",
                "Beziehungsschritte vom Objekt zu seinen Gegenobjekten.",
            ),
            param!(
                "counterpart_selector",
                "Which reached objects are counterparts.",
                "Welche erreichten Objekte Gegenobjekte sind.",
            ),
            param!(
                "container_selector",
                "The containers, such as storeys or spaces.",
                "Die Container, etwa Geschosse oder Räume.",
            ),
            CONTAINER_RELATIONSHIP,
            LEVEL_PROPERTY,
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.selector-conformance",
        label: en_de("Selector conformance", "Selektorkonformität"),
        help: en_de(
            "Requires each selected object to satisfy a requirement selector, such as an agreed list of property combinations, grouping failing objects by their values. An object the requirement cannot decide is not evaluated.",
            "Verlangt, dass jedes gewählte Objekt einen Anforderungsselektor erfüllt, etwa eine vereinbarte Liste von Eigenschaftskombinationen, und fasst nicht erfüllende Objekte nach ihren Werten zusammen. Ein Objekt, das die Anforderung nicht entscheiden kann, bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "requirement",
                "The selector each object must satisfy.",
                "Der Selektor, den jedes Objekt erfüllen muss.",
            ),
            param!(
                "message",
                "Message replacing the default, followed by the values found.",
                "Meldung anstelle der Standardmeldung, gefolgt von den gefundenen Werten.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.shelf-capacity",
        label: en_de("Shelf capacity", "Regalkapazität"),
        help: en_de(
            "Lays out parallel bands of shelving in each selected space, clear of the zones at doors and openings, and requires at least the given running metres over all tiers. A door or opening whose spaces cannot be read, or whose type is undecided, leaves the space not evaluated.",
            "Plant parallele Regalreihen in jedem gewählten Raum, frei von den Zonen an Türen und Öffnungen, und verlangt mindestens die angegebenen laufenden Meter über alle Fachebenen. Eine Tür oder Öffnung, deren Räume nicht lesbar sind oder deren Typ unentschieden ist, lässt den Raum unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum_running_metres",
                "Running metres of shelving the space must hold, all tiers counted.",
                "Laufende Meter Regal, die der Raum aufnehmen muss, alle Fachebenen gezählt.",
            ),
            param!(
                "shelf_depth_metres",
                "Depth of one band of shelving in metres.",
                "Tiefe einer Regalreihe in Metern.",
            ),
            param!(
                "horizontal_spacing_metres",
                "Width in metres of the aisle serving the bands.",
                "Breite des Bediengangs zwischen den Regalreihen in Metern.",
            ),
            param!(
                "vertical_spacing_metres",
                "Height in metres of one tier.",
                "Höhe einer Fachebene in Metern.",
            ),
            param!(
                "bottom_elevation_metres",
                "Height in metres above the floor where the lowest tier starts.",
                "Höhe über dem Boden in Metern, auf der die unterste Fachebene beginnt.",
            ),
            param!(
                "top_elevation_metres",
                "Height in metres above the floor where the shelving ends.",
                "Höhe über dem Boden in Metern, auf der das Regal endet.",
            ),
            param!(
                "door_clearance_metres",
                "How far in metres the clear zone at a door or opening reaches in every direction.",
                "Wie weit in Metern die freizuhaltende Zone an einer Tür oder Öffnung in alle Richtungen reicht.",
            ),
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.slab-contact",
        label: en_de("Slab contact", "Auflagerkontakt"),
        help: en_de(
            "Requires at least a share of each selected subject's top or bottom face to rest on a counterpart, within gap and intersection tolerances, optionally skipping the top or bottom storey. An unmeasured candidate leaves the subject not evaluated, and an undecided counterpart leaves a shortfall not evaluated.",
            "Verlangt, dass mindestens ein Anteil der Ober- oder Unterseite jedes gewählten Bauteils innerhalb von Spalt- und Durchdringungstoleranzen auf einem Gegenbauteil aufliegt, optional ohne das oberste oder unterste Geschoss. Ein nicht gemessener Kandidat lässt das Bauteil unbewertet, und ein unentschiedenes Gegenbauteil lässt eine Unterschreitung unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum_contact_ratio",
                "Least share of the face that must be in contact.",
                "Geringster Anteil der Fläche, der aufliegen muss.",
            ),
            param!(
                "contact_side",
                "`above` or `below`: which face is checked.",
                "`above` oder `below`: welche Fläche geprüft wird.",
            ),
            param!(
                "maximum_gap_metres",
                "Largest gap in metres that still counts as contact.",
                "Größter Spalt in Metern, der noch als Kontakt gilt.",
            ),
            param!(
                "maximum_intersection_metres",
                "Largest overlap in metres that still counts as contact.",
                "Größte Überschneidung in Metern, die noch als Kontakt gilt.",
            ),
            param!(
                "minimum_polygon_area_square_metres",
                "Contact patches smaller than this, in square metres, are ignored.",
                "Kontaktflächen kleiner als dieser Wert in Quadratmetern werden ignoriert.",
            ),
            param!(
                "counterparts",
                "What the face may rest on; every other object by default.",
                "Worauf die Fläche aufliegen darf; standardmäßig jedes andere Objekt.",
            ),
            param!(
                "skip_top_storey",
                "Leave out subjects on the highest storey of their source.",
                "Bauteile im obersten Geschoss ihrer Quelle auslassen.",
            ),
            param!(
                "skip_bottom_storey",
                "Leave out subjects on the lowest storey of their source.",
                "Bauteile im untersten Geschoss ihrer Quelle auslassen.",
            ),
            param!(
                "storey_selector",
                "The storeys, ordered by their elevation.",
                "Die Geschosse, nach ihrer Höhenlage geordnet.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.slab-stack-spacing",
        label: en_de("Slab stack spacing", "Abstand übereinanderliegender Decken"),
        help: en_de(
            "Checks each selected slab against the next slab up it stacks with: top-to-top, bottom-to-bottom and clear distances within bounds and, optionally, equal to the stack's prevailing distance. A distance or overlap straddling a bound, or an unmeasurable slab, leaves the slabs concerned not evaluated.",
            "Prüft jede gewählte Decke gegen die nächsthöhere Decke ihres Stapels: Abstände Oberkante zu Oberkante, Unterkante zu Unterkante und lichter Abstand innerhalb von Grenzen und optional gleich dem vorherrschenden Abstand des Stapels. Ein Abstand oder eine Überlappung, die eine Grenze überspannt, oder eine nicht messbare Decke lässt die betroffenen Decken unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum_overlap_ratio",
                "Least share of the smaller footprint two slabs must overlap to stack.",
                "Geringster Anteil des kleineren Grundrisses, um den sich zwei Decken überlappen müssen, um einen Stapel zu bilden.",
            ),
            param!(
                "top_to_top_minimum",
                "Least distance from a slab's top to the next slab's top.",
                "Geringster Abstand von der Oberkante einer Decke zur Oberkante der nächsten.",
            ),
            param!(
                "top_to_top_maximum",
                "Greatest distance from a slab's top to the next slab's top.",
                "Größter Abstand von der Oberkante einer Decke zur Oberkante der nächsten.",
            ),
            param!(
                "bottom_to_bottom_minimum",
                "Least distance from a slab's bottom to the next slab's bottom.",
                "Geringster Abstand von der Unterkante einer Decke zur Unterkante der nächsten.",
            ),
            param!(
                "bottom_to_bottom_maximum",
                "Greatest distance from a slab's bottom to the next slab's bottom.",
                "Größter Abstand von der Unterkante einer Decke zur Unterkante der nächsten.",
            ),
            param!(
                "top_to_bottom_minimum",
                "Least clear gap from a slab's top to the next slab's underside.",
                "Geringster lichter Abstand von der Oberkante einer Decke zur Unterseite der nächsten.",
            ),
            param!(
                "top_to_bottom_maximum",
                "Greatest clear gap from a slab's top to the next slab's underside.",
                "Größter lichter Abstand von der Oberkante einer Decke zur Unterseite der nächsten.",
            ),
            param!(
                "consistent",
                "Measures that must equal the stack's prevailing one: `top_to_top`, `bottom_to_bottom`, `top_to_bottom`.",
                "Maße, die dem vorherrschenden des Stapels entsprechen müssen: `top_to_top`, `bottom_to_bottom`, `top_to_bottom`.",
            ),
            param!(
                "tolerance",
                "Allowed difference from the prevailing distance; 1 mm by default.",
                "Zulässige Abweichung vom vorherrschenden Abstand; standardmäßig 1 mm.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.space-boundary-coverage",
        label: en_de(
            "Space boundary coverage",
            "Abdeckung durch Raumbegrenzungen",
        ),
        help: en_de(
            "Measures how much of each selected space's body surface its declared space boundaries cover, leave uncovered or cover twice. A boundary lying on no face is a finding; a missing or curved body, or an unreadable boundary surface, is not evaluated.",
            "Misst, wie viel der Körperoberfläche jedes gewählten Raums seine angegebenen Raumbegrenzungen abdecken, freilassen oder doppelt abdecken. Eine Begrenzung auf keiner Fläche ist ein Befund; ein fehlender oder gekrümmter Körper oder eine nicht lesbare Begrenzungsfläche bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "minimum_covered_share",
                "Least share of the surface, from 0 to 1, the boundaries must cover.",
                "Geringster Anteil der Oberfläche von 0 bis 1, den die Begrenzungen abdecken müssen.",
            ),
            param!(
                "maximum_uncovered_area",
                "Largest area of the surface the boundaries may leave uncovered.",
                "Größte Fläche der Oberfläche, die die Begrenzungen freilassen dürfen.",
            ),
            param!(
                "maximum_overlap_area",
                "Largest area of the surface two or more boundaries may cover together.",
                "Größte Fläche der Oberfläche, die zwei oder mehr Begrenzungen gemeinsam abdecken dürfen.",
            ),
            param!(
                "plane_tolerance",
                "How far from a face plane of the body a boundary surface may lie and still count; 0 by default.",
                "Wie weit eine Begrenzungsfläche von einer Flächenebene des Körpers entfernt liegen darf und noch zählt; standardmäßig 0.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.space-connection",
        label: en_de("Space connection", "Raumverbindungen"),
        help: en_de(
            "Checks each selected space against its connection rows: direct access through doors or openings to other spaces required or forbidden, and a direct exit to the outside. An element of undecided type, or whose spaces cannot be read, leaves an answer it could change not evaluated.",
            "Prüft jeden gewählten Raum gegen seine Verbindungszeilen: direkter Zugang über Türen oder Öffnungen zu anderen Räumen gefordert oder verboten sowie ein direkter Ausgang ins Freie. Ein Element unentschiedenen Typs oder mit nicht lesbaren Räumen lässt eine Antwort, die es ändern könnte, unbewertet.",
        ),
        parameters: &[
            param!(
                "connections",
                "Rows: the spaces they apply to, the target spaces, required or forbidden access, the element type and the exit.",
                "Zeilen: die betroffenen Räume, die Zielräume, geforderter oder verbotener Zugang, der Elementtyp und der Ausgang.",
            ),
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.space-distance",
        label: en_de("Space distance", "Abstand zwischen Räumen"),
        help: en_de(
            "Checks each selected space against its distance rows: the straight, closest or walking distance to the nearest destination space lies within bounds. A route that cannot be measured decides only what it cannot change; anything else is not evaluated.",
            "Prüft jeden gewählten Raum gegen seine Abstandszeilen: der direkte, kürzeste oder begehbare Abstand zum nächsten Zielraum liegt innerhalb der Grenzen. Ein nicht messbarer Weg entscheidet nur, was er nicht ändern kann; alles andere bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "distances",
                "Rows: start and destination spaces, the measure, same storey, direct access and bounds in metres.",
                "Zeilen: Start- und Zielräume, das Maß, gleiches Geschoss, direkter Zugang und Grenzen in Metern.",
            ),
            param!(
                "storey_path",
                "Relationship steps climbed from a space to its storeys.",
                "Beziehungsschritte, über die von einem Raum zu seinen Geschossen aufgestiegen wird.",
            ),
            param!("storey_selector", "The storeys.", "Die Geschosse."),
            ACCESS_PATH,
            DOOR_SELECTOR,
            OPENING_SELECTOR,
            SPACE_SELECTOR,
            param!(
                "walking_radius",
                "Radius in metres of the body a walking row routes.",
                "Radius des Körpers in Metern, für den eine Gehzeile den Weg sucht.",
            ),
            param!(
                "walking_height",
                "Height in metres of the body a walking row routes.",
                "Höhe des Körpers in Metern, für den eine Gehzeile den Weg sucht.",
            ),
            param!(
                "walking_step",
                "Largest step in metres the walking body crosses.",
                "Größte Stufe in Metern, die der gehende Körper überwindet.",
            ),
            param!(
                "walking_slope",
                "Largest slope, as rise over run, the walking body climbs; 0 (level) by default.",
                "Größte Neigung als Höhe zu Länge, die der gehende Körper bewältigt; standardmäßig 0 (eben).",
            ),
            STAIR_SELECTOR,
            RAMP_SELECTOR,
            LIFT_SELECTOR,
            STAIR_LENGTH,
            VERTICAL_FACTOR,
        ],
    },
    CapabilityText {
        id: "axioval:capability.space-validation",
        label: en_de("Space validation", "Raumprüfung"),
        help: en_de(
            "Checks each selected space for duplicates, insufficient height, uncovered boundaries, contained bodies, intersections, uncovered top and bottom caps and unallocated storey area. A sub-check the service cannot measure is not evaluated without affecting the others.",
            "Prüft jeden gewählten Raum auf Duplikate, unzureichende Höhe, nicht abgedeckte Begrenzungen, enthaltene Körper, Durchdringungen, nicht abgedeckte Decken- und Bodenflächen sowie keinem Raum zugeordnete Geschossfläche. Eine Teilprüfung, die der Dienst nicht messen kann, bleibt unbewertet, ohne die anderen zu beeinflussen.",
        ),
        parameters: &[
            param!(
                "required_height_metres",
                "Least clear height of the space in metres.",
                "Geringste lichte Raumhöhe in Metern.",
            ),
            param!(
                "uncovered_segment_length_metres",
                "Boundary runs at least this long, in metres, that no element covers are findings.",
                "Begrenzungsabschnitte mindestens dieser Länge in Metern, die kein Bauteil abdeckt, sind Befunde.",
            ),
            param!(
                "check_top_cap",
                "Check that the space is covered from above.",
                "Prüfen, ob der Raum nach oben abgeschlossen ist.",
            ),
            param!(
                "check_bottom_cap",
                "Check that the space is covered from below.",
                "Prüfen, ob der Raum nach unten abgeschlossen ist.",
            ),
            param!(
                "check_unallocated_area",
                "Check the storey floor area no space covers.",
                "Die Geschossfläche prüfen, die keinem Raum zugeordnet ist.",
            ),
            param!(
                "maximum_unallocated_area_square_metres",
                "Largest connected unallocated region in square metres.",
                "Größte zusammenhängende nicht zugeordnete Fläche in Quadratmetern.",
            ),
            param!(
                "tolerance_metres",
                "Tolerance in metres for the height and for overlaps counted as contact; 0.005 by default.",
                "Toleranz in Metern für die Höhe und für Überlappungen, die als Kontakt gelten; standardmäßig 0,005.",
            ),
            param!(
                "maximum_unallocated_share",
                "Largest share, from 0 to 1, of a storey's gross floor area its unallocated regions may cover together.",
                "Größter Anteil von 0 bis 1 an der Bruttogeschossfläche, den die nicht zugeordneten Flächen gemeinsam einnehmen dürfen.",
            ),
            param!(
                "top_cap_elements",
                "Elements that may cap a space from above; the declared slabs and roofs by default.",
                "Bauteile, die einen Raum nach oben abschließen können; standardmäßig die angegebenen Decken und Dächer.",
            ),
            param!(
                "bottom_cap_elements",
                "Elements that may cap a space from below; the declared slabs by default.",
                "Bauteile, die einen Raum nach unten abschließen können; standardmäßig die angegebenen Decken.",
            ),
            param!(
                "boundary_elements",
                "Elements that bound a space; every body that is not a space by default.",
                "Bauteile, die einen Raum begrenzen; standardmäßig jeder Körper, der kein Raum ist.",
            ),
            param!(
                "intersection_elements",
                "Elements a space must not intersect; every other body by default.",
                "Bauteile, die ein Raum nicht durchdringen darf; standardmäßig jeder andere Körper.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.stair-geometry",
        label: en_de("Stair geometry", "Treppengeometrie"),
        help: en_de(
            "Measures each selected flight, or each whole stair, and checks risers, goings, step length, nosings, winders, width, landings, headroom, handrails, end spaces, tactile strips and clear widths. A measurement straddling its bound, or a flight the service cannot measure, is not evaluated.",
            "Misst jeden gewählten Treppenlauf oder jede ganze Treppe und prüft Steigungen, Auftritte, Schrittmaß, Unterschneidungen, gewendelte Stufen, Breite, Podeste, Kopfhöhe, Handläufe, Bewegungsflächen an den Enden, taktile Aufmerksamkeitsfelder und lichte Breiten. Ein Messwert, der seine Grenze überspannt, oder ein vom Dienst nicht messbarer Lauf bleibt unbewertet.",
        ),
        parameters: &[
            param!(
                "riser_minimum",
                "Least height of every riser.",
                "Geringste Höhe jeder Steigung.",
            ),
            param!(
                "riser_maximum",
                "Greatest height of every riser.",
                "Größte Höhe jeder Steigung.",
            ),
            param!(
                "going_minimum",
                "Least going of every tread.",
                "Geringster Auftritt jeder Stufe.",
            ),
            param!(
                "going_maximum",
                "Greatest going of every tread.",
                "Größter Auftritt jeder Stufe.",
            ),
            param!(
                "step_length_minimum",
                "Least step length, twice the riser plus the going, for every tread.",
                "Geringstes Schrittmaß, zwei Steigungen plus ein Auftritt, für jede Stufe.",
            ),
            param!(
                "step_length_maximum",
                "Greatest step length, twice the riser plus the going, for every tread.",
                "Größtes Schrittmaß, zwei Steigungen plus ein Auftritt, für jede Stufe.",
            ),
            param!(
                "nosing_minimum",
                "Least nosing, how far each tread reaches over the one below.",
                "Geringste Unterschneidung, wie weit jede Stufe über die darunterliegende ragt.",
            ),
            param!(
                "nosing_maximum",
                "Greatest nosing, how far each tread reaches over the one below.",
                "Größte Unterschneidung, wie weit jede Stufe über die darunterliegende ragt.",
            ),
            param!(
                "minimum_risers",
                "Least number of risers; may be computed per object.",
                "Geringste Anzahl der Steigungen; kann je Objekt berechnet werden.",
            ),
            param!(
                "maximum_risers",
                "Greatest number of risers; may be computed per object.",
                "Größte Anzahl der Steigungen; kann je Objekt berechnet werden.",
            ),
            param!(
                "maximum_rise",
                "Greatest rise of the flight; may be computed per object.",
                "Größte Höhe des Laufs; kann je Objekt berechnet werden.",
            ),
            param!(
                "riser_tolerance",
                "Largest difference between the flight's highest and lowest riser.",
                "Größter Unterschied zwischen der höchsten und der niedrigsten Steigung des Laufs.",
            ),
            param!(
                "going_tolerance",
                "Largest difference between the flight's largest and smallest going.",
                "Größter Unterschied zwischen dem größten und dem kleinsten Auftritt des Laufs.",
            ),
            param!(
                "walking_line_offset",
                "Distance of a turning flight's walking line from the side it turns towards; the centre line without it.",
                "Abstand der Lauflinie eines gewendelten Laufs von der Innenseite der Wendelung; ohne Angabe die Mittellinie.",
            ),
            param!(
                "winder_angle_maximum",
                "Greatest plan angle between consecutive nosings.",
                "Größter Grundrisswinkel zwischen aufeinanderfolgenden Stufenvorderkanten.",
            ),
            param!(
                "winder_angle_minimum",
                "Least angle of every winder; straight treads are skipped.",
                "Geringster Winkel jeder gewendelten Stufe; gerade Stufen werden übersprungen.",
            ),
            param!(
                "forbid_open_risers",
                "Every open riser is a finding.",
                "Jede offene Stufe ohne Setzstufe ist ein Befund.",
            ),
            param!(
                "handrail_extension_from",
                "`nosing` (default) or `riser`: where handrail extensions are measured from.",
                "`nosing` (Standard) oder `riser`: wovon aus die Handlaufverlängerungen gemessen werden.",
            ),
            MINIMUM_HEADROOM,
            HEADROOM_OBSTACLES,
            WIDTH_MINIMUM,
            WIDTH_MAXIMUM,
            LANDING_OBJECTS,
            LANDING_DEPTH_MINIMUM,
            LANDING_WIDTH_MINIMUM,
            LANDING_AT_LEAST_WALKING_WIDTH,
            LANDINGS_REQUIRED,
            MINIMUM_HEADROOM_BELOW,
            HEADROOM_BELOW_SPACES,
            HANDRAIL_OBJECTS,
            HANDRAIL_REACH_ACROSS,
            HANDRAIL_REACH_ABOVE,
            HANDRAIL_HEIGHT_MINIMUM,
            HANDRAIL_HEIGHT_MAXIMUM,
            HANDRAIL_EXTENSION_MINIMUM,
            HANDRAIL_EXTENSION_MAXIMUM,
            HANDRAIL_GAP_MAXIMUM,
            HANDRAIL_SIDES,
            HANDRAIL_BOTH_SIDES_ABOVE_WIDTH,
            LANDING_DOORS,
            LANDING_DOOR_HEIGHT,
            LANDING_DOOR_SWING,
            END_SPACE_DEPTH,
            END_SPACE_WIDTH,
            END_SPACE_HEIGHT,
            END_SPACE_OBSTACLES,
            CLEAR_WIDTH_MINIMUM,
            CLEAR_WIDTH_OBSTACLES,
            CLEAR_WIDTH_BAND_FROM,
            CLEAR_WIDTH_BAND_TO,
            param!(
                "stair_path",
                "Relationship steps from a whole stair to its parts, to check whole stairs.",
                "Beziehungsschritte von einer ganzen Treppe zu ihren Teilen, um ganze Treppen zu prüfen.",
            ),
            param!(
                "stair_flights",
                "Which reached parts are flights; required with `stair_path`.",
                "Welche erreichten Teile Treppenläufe sind; erforderlich mit `stair_path`.",
            ),
            param!(
                "maximum_total_rise",
                "Greatest rise of the whole stair, from its lowest flight's base to its highest flight's top.",
                "Größte Gesamthöhe der Treppe, vom Fuß des untersten bis zur Oberkante des obersten Laufs.",
            ),
            param!(
                "handrail_continuous_across_landings",
                "The handrail along each side must continue across every landing between flights.",
                "Der Handlauf jeder Seite muss über jedes Zwischenpodest hinweg durchlaufen.",
            ),
            param!(
                "handrail_break_doors",
                "Doors that may break the handrail where they stand at a landing.",
                "Türen, die den Handlauf dort unterbrechen dürfen, wo sie an einem Podest liegen.",
            ),
            param!(
                "tactile_objects",
                "The tactile warning surfaces, such as coverings of a tactile type.",
                "Die taktilen Aufmerksamkeitsfelder, etwa Bodenbeläge eines taktilen Typs.",
            ),
            param!(
                "tactile_offset",
                "How far before the first riser and beyond the last the tactile strip starts; 0 for directly at it.",
                "Wie weit vor der ersten und hinter der letzten Steigung das taktile Feld beginnt; 0 für unmittelbar daran.",
            ),
            param!(
                "tactile_depth",
                "Depth of the tactile strip along the walking direction.",
                "Tiefe des taktilen Felds in Laufrichtung.",
            ),
            param!(
                "tactile_on_intermediate_landings",
                "In whole-stair mode, also require tactile strips on the landings between flights.",
                "Bei ganzen Treppen taktile Felder auch auf den Zwischenpodesten verlangen.",
            ),
            param!(
                "landing_clear_width_minimum",
                "Least clear width of the landing at each end of the flight.",
                "Geringste lichte Breite des Podests an jedem Ende des Laufs.",
            ),
            param!(
                "total_clear_width_minimum",
                "Least of the clear widths of the flights and the landings between them.",
                "Geringste der lichten Breiten der Läufe und der Podeste dazwischen.",
            ),
        ],
    },
    CapabilityText {
        id: "axioval:capability.table-allocation",
        label: en_de("Table allocation", "Zuordnung zu Tabellenzeilen"),
        help: en_de(
            "Assigns each selected object to one row of a table by key patterns, such as a room programme, and checks every row's count and area per anchor, source or project. A tie, an undecided key or an undecided selection leaves the rows concerned not evaluated unless they are already exceeded.",
            "Ordnet jedes gewählte Objekt über Schlüsselmuster genau einer Tabellenzeile zu, etwa eines Raumprogramms, und prüft Anzahl und Fläche jeder Zeile je Bezugsobjekt, Quelle oder Projekt. Ein Gleichstand, ein unentschiedener Schlüssel oder eine unentschiedene Auswahl lässt die betroffenen Zeilen unbewertet, sofern sie nicht bereits überschritten sind.",
        ),
        parameters: &[
            param!(
                "rows",
                "Rows: key and anchor patterns, a label, the required count and the area with its tolerances.",
                "Zeilen: Schlüssel- und Bezugsmuster, eine Bezeichnung, die geforderte Anzahl und die Fläche mit ihren Toleranzen.",
            ),
            param!(
                "mode",
                "`first` (default) or `most_specific`: which matching row an object is assigned to.",
                "`first` (Standard) oder `most_specific`: welcher passenden Zeile ein Objekt zugeordnet wird.",
            ),
            param!(
                "key_1",
                "Property the rows' `key_1` patterns match.",
                "Eigenschaft, mit der die Muster `key_1` der Zeilen abgeglichen werden.",
            ),
            param!(
                "key_2",
                "Property the rows' `key_2` patterns match.",
                "Eigenschaft, mit der die Muster `key_2` der Zeilen abgeglichen werden.",
            ),
            param!(
                "key_3",
                "Property the rows' `key_3` patterns match.",
                "Eigenschaft, mit der die Muster `key_3` der Zeilen abgeglichen werden.",
            ),
            param!(
                "key_4",
                "Property the rows' `key_4` patterns match.",
                "Eigenschaft, mit der die Muster `key_4` der Zeilen abgeglichen werden.",
            ),
            param!(
                "case_sensitive",
                "Whether patterns respect case; true by default.",
                "Ob Muster Groß- und Kleinschreibung beachten; standardmäßig true.",
            ),
            param!(
                "area_property",
                "Area property read instead of measuring footprints.",
                "Flächeneigenschaft, die gelesen wird, statt Grundrisse zu messen.",
            ),
            param!(
                "area_mode",
                "`sum` (default): a row's objects' areas are summed; `each`: each object's own area is a match condition.",
                "`sum` (Standard): die Flächen der Objekte einer Zeile werden summiert; `each`: die Fläche jedes Objekts ist eine Zuordnungsbedingung.",
            ),
            param!(
                "anchor_selector",
                "Anchors forming the groups, such as storeys.",
                "Bezugsobjekte, die die Gruppen bilden, etwa Geschosse.",
            ),
            param!(
                "anchor_key",
                "Anchor property the rows' `anchor` patterns match, such as a storey's name.",
                "Eigenschaft des Bezugsobjekts, mit der die Muster `anchor` der Zeilen abgeglichen werden, etwa der Geschossname.",
            ),
            param!(
                "across_sources",
                "Without anchors, one group for the whole project; per source by default.",
                "Ohne Bezugsobjekte eine Gruppe für das gesamte Projekt; standardmäßig je Quelle.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.triangle-count",
        label: en_de("Triangle count", "Dreiecksanzahl"),
        help: en_de(
            "Requires each selected object's mesh to hold at most a number of triangles. The count depends on the tessellation; an unmeasured object is not evaluated.",
            "Verlangt, dass das Netz jedes gewählten Objekts höchstens eine bestimmte Anzahl Dreiecke enthält. Die Anzahl hängt von der Tessellierung ab; ein nicht gemessenes Objekt bleibt unbewertet.",
        ),
        parameters: &[param!(
            "maximum",
            "Greatest number of triangles.",
            "Größte Anzahl an Dreiecken.",
        )],
    },
    CapabilityText {
        id: "axioval:capability.unclassified-object",
        label: en_de("Unclassified object", "Nicht klassifiziertes Objekt"),
        help: en_de(
            "Reports every selected object to which the ruleset's classification assigns no class. An object whose deciding row cannot be decided is not evaluated, and an undeclared classification leaves the rule not evaluated.",
            "Meldet jedes gewählte Objekt, dem die Klassifikation des Regelwerks keine Klasse zuweist. Ein Objekt, dessen maßgebliche Zeile nicht entschieden werden kann, bleibt unbewertet, und eine nicht deklarierte Klassifikation lässt die Regel unbewertet.",
        ),
        parameters: &[param!(
            "classification",
            "Id of the ruleset's classification.",
            "Kennung der Klassifikation des Regelwerks.",
        )],
    },
    CapabilityText {
        id: "axioval:capability.unique-value",
        label: en_de("Unique value", "Eindeutiger Wert"),
        help: en_de(
            "Requires a property not to repeat among the selected objects of one source, of the project, or of one related scope such as a storey. A missing value is a finding unless `require_value` is false; an undecided value leaves the object not evaluated.",
            "Verlangt, dass sich eine Eigenschaft unter den gewählten Objekten einer Quelle, des Projekts oder eines verbundenen Bereichs wie eines Geschosses nicht wiederholt. Ein fehlender Wert ist ein Befund, sofern `require_value` nicht false ist; ein unentschiedener Wert lässt das Objekt unbewertet.",
        ),
        parameters: &[
            param!(
                "property",
                "The property whose values must be unique.",
                "Die Eigenschaft, deren Werte eindeutig sein müssen.",
            ),
            param!(
                "trim",
                "Drop surrounding whitespace before comparing text; true by default.",
                "Umgebende Leerzeichen vor dem Textvergleich entfernen; standardmäßig true.",
            ),
            param!(
                "case_sensitive",
                "Whether text compares by case; false by default.",
                "Ob Text Groß- und Kleinschreibung unterscheidet; standardmäßig false.",
            ),
            param!(
                "require_value",
                "A missing value is a finding; true by default.",
                "Ein fehlender Wert ist ein Befund; standardmäßig true.",
            ),
            param!(
                "across_sources",
                "One scope for the whole project; per source by default.",
                "Ein Bereich für das gesamte Projekt; standardmäßig je Quelle.",
            ),
            RELATIONSHIP,
            DIRECTION,
            FOLLOW_CHAIN,
            PATH,
            SKIP_ABSENT_RELATIONSHIP_ENDS,
            TOLERANCE,
            RELATIVE_TOLERANCE,
            DECIMALS,
        ],
    },
    CapabilityText {
        id: "axioval:capability.wall-spacing",
        label: en_de("Wall spacing", "Wandabstände"),
        help: en_de(
            "Checks the parallel walls or beams of each selected storey: pairs closer than a minimum, and footprint area left outside the bands that pairs at most a maximum apart enclose. A pair that may or may not be parallel and facing, or an area straddling its bound, is not evaluated.",
            "Prüft die parallelen Wände oder Träger jedes gewählten Geschosses: Paare, die näher als ein Mindestabstand liegen, und Grundrissfläche außerhalb der Streifen, die Paare mit höchstens einem Höchstabstand einschließen. Ein Paar, das möglicherweise parallel und gegenüberliegend ist, oder eine Fläche, die ihre Grenze überspannt, bleibt unbewertet.",
        ),
        parameters: &[
            param!("members", "The walls or beams.", "Die Wände oder Träger."),
            param!(
                "member_path",
                "Relationship steps from the storey to its members.",
                "Beziehungsschritte vom Geschoss zu seinen Wänden oder Trägern.",
            ),
            param!(
                "angle_tolerance",
                "How far from parallel two long axes may be; in [0°, 45°).",
                "Wie weit zwei Längsachsen von parallel abweichen dürfen; in [0°, 45°).",
            ),
            param!(
                "minimum",
                "Least plan distance between a parallel pair.",
                "Geringster Grundrissabstand zwischen einem parallelen Paar.",
            ),
            param!(
                "maximum",
                "Largest distance at which a parallel pair encloses a band.",
                "Größter Abstand, bei dem ein paralleles Paar einen Streifen einschließt.",
            ),
            param!(
                "footprints",
                "With `maximum`: objects whose footprints make the storey's gross footprint, such as its slabs.",
                "Bei `maximum`: Objekte, deren Grundrisse die Bruttogrundfläche des Geschosses bilden, etwa seine Decken.",
            ),
            param!(
                "footprint_path",
                "With `maximum`: relationship steps from the storey to those objects.",
                "Bei `maximum`: Beziehungsschritte vom Geschoss zu diesen Objekten.",
            ),
            param!(
                "uncovered_above",
                "With `maximum`: area of a footprint that may lie outside every band.",
                "Bei `maximum`: Fläche eines Grundrisses, die außerhalb jedes Streifens liegen darf.",
            ),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::{CAPABILITY_TEXTS, LocalizedText};
    use crate::register_builtins;
    use axioval_engine::CapabilityRegistry;

    /// A text that may read alike in both languages: one identifier-like word.
    fn identifier_like(text: &str) -> bool {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    }

    fn check(context: &str, texts: &[LocalizedText], problems: &mut Vec<String>) {
        let [en, de] = texts else {
            problems.push(format!("{context}: not exactly en and de"));
            return;
        };
        if en.language != "en" || de.language != "de" {
            problems.push(format!("{context}: languages are not en, de"));
        }
        for text in texts {
            if text.text.trim().is_empty() {
                problems.push(format!("{context}: empty {} text", text.language));
            }
        }
        if en.text == de.text && !identifier_like(en.text) {
            problems.push(format!(
                "{context}: German text equals the English `{}`",
                en.text
            ));
        }
    }

    #[test]
    fn every_registered_capability_has_texts_for_its_parameters() {
        let registry =
            register_builtins(CapabilityRegistry::default()).expect("built-ins register");
        let registered: Vec<&str> = registry.ids().collect();
        let documented: Vec<&str> = CAPABILITY_TEXTS.iter().map(|text| text.id).collect();

        assert!(
            documented.windows(2).all(|pair| pair[0] < pair[1]),
            "entries must be strictly sorted by id"
        );
        let missing: Vec<&&str> = registered
            .iter()
            .filter(|id| !documented.contains(id))
            .collect();
        let unknown: Vec<&&str> = documented
            .iter()
            .filter(|id| !registered.contains(id))
            .collect();
        assert!(
            missing.is_empty(),
            "capabilities without texts: {missing:?}"
        );
        assert!(
            unknown.is_empty(),
            "texts of unregistered capabilities: {unknown:?}"
        );

        let mut problems = Vec::new();
        for text in CAPABILITY_TEXTS {
            let capability = registry.get(text.id).expect("registered");
            let expected: Vec<String> = capability
                .parameters()
                .into_iter()
                .map(|parameter| parameter.name)
                .collect();
            let written: Vec<&str> = text
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect();
            if written != expected {
                problems.push(format!(
                    "{}: parameters {written:?} differ from {expected:?}",
                    text.id
                ));
            }
            check(&format!("{} label", text.id), &text.label, &mut problems);
            for label in &text.label {
                if label.text.ends_with('.') {
                    problems.push(format!("{} label ends with a period", text.id));
                }
            }
            check(&format!("{} help", text.id), &text.help, &mut problems);
            for parameter in text.parameters {
                check(
                    &format!("{} parameter {}", text.id, parameter.name),
                    parameter.help,
                    &mut problems,
                );
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
