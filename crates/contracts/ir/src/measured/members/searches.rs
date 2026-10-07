//! The member lists the search capabilities' templates judge (#286): each
//! a search kept in built-in code, answering per checked object what it
//! found (a fit, a route, a distance, an allocation), which the template
//! decides over.

use super::super::registry::en_de;
use super::super::{
    LocalizedText, MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind,
    MeasuredSubject,
};
use super::{AREA, MemberDescriptor, MemberFieldKind, RATIO, TRUTH, field};

const OBJECTS: MemberFieldKind = MemberFieldKind::Objects;

const fn objects(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help,
    }
}

const fn length(
    key: &'static str,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required,
        default: None,
        help,
    }
}

const fn path(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help,
    }
}

/// The free-floor search as `free-floor-circle` and `free-floor-rectangle`
/// declare it, every parameter under the capabilities' own name.
const FREE_FLOOR: [MeasuredParameter; 17] = [
    MeasuredParameter {
        key: "shape",
        kind: MeasuredParameterKind::Choice {
            options: &["circle", "rectangle"],
        },
        required: true,
        default: None,
        help: &en_de(
            "The shape searched for: a circle of `diameter_metres`, or a rectangle \
             `width_metres` by `length_metres`, each `height_metres` high.",
            "Die gesuchte Form: ein Kreis mit `diameter_metres` oder ein Rechteck \
             `width_metres` mal `length_metres`, jeweils `height_metres` hoch.",
        ),
    },
    length(
        "diameter_metres",
        false,
        &en_de("The circle's diameter.", "Der Durchmesser des Kreises."),
    ),
    length(
        "width_metres",
        false,
        &en_de("The rectangle's width.", "Die Breite des Rechtecks."),
    ),
    length(
        "length_metres",
        false,
        &en_de("The rectangle's length.", "Die Länge des Rechtecks."),
    ),
    length(
        "height_metres",
        true,
        &en_de("The shape's height.", "Die Höhe der Form."),
    ),
    MeasuredParameter {
        key: "orientation",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rectangle's orientation: `any`, every rotation counting.",
            "Die Ausrichtung des Rechtecks: `any`, jede Drehung zählt.",
        ),
    },
    objects(
        "obstacles",
        &en_de(
            "What obstructs the floor; every other object without it. Objects the \
             selection cannot decide can only keep a proof of absence open.",
            "Was den Boden verstellt; ohne Angabe jedes andere Objekt. Von der Auswahl \
             unentschiedene Objekte können nur einen Nachweis des Fehlens offenhalten.",
        ),
    ),
    length(
        "band_from_metres",
        false,
        &en_de(
            "Where above the floor obstacles start to count.",
            "Ab welcher Höhe über dem Boden Hindernisse zählen.",
        ),
    ),
    length(
        "band_to_metres",
        false,
        &en_de(
            "Where above the floor obstacles stop counting; the shape's height without it.",
            "Bis zu welcher Höhe über dem Boden Hindernisse zählen; ohne Angabe die Höhe \
             der Form.",
        ),
    ),
    path(
        "merge_path",
        &en_de(
            "The relationship steps to the spaces searched with the space.",
            "Die Beziehungsschritte zu den mit dem Raum durchsuchten Räumen.",
        ),
    ),
    objects(
        "subtract_door_swings",
        &en_de(
            "The doors whose swings obstruct the floor.",
            "Die Türen, deren Aufschlag den Boden verstellt.",
        ),
    ),
    length(
        "entrance_path_width",
        false,
        &en_de(
            "How wide a path from an entrance must reach the shape.",
            "Wie breit ein Weg von einem Zugang die Form erreichen muss.",
        ),
    ),
    length(
        "entrance_tolerance_metres",
        false,
        &en_de(
            "How much farther than half the path's width an entrance may lie from it \
             (0.05 m by default).",
            "Wie viel weiter als die halbe Wegbreite ein Zugang vom Weg liegen darf \
             (standardmäßig 0,05 m).",
        ),
    ),
    path(
        "access_path",
        &en_de(
            "The relationship steps from a door or opening to the spaces it opens onto.",
            "Die Beziehungsschritte von einer Tür oder Öffnung zu den Räumen, in die sie \
             führt.",
        ),
    ),
    objects(
        "door_selector",
        &en_de(
            "The doors that are entrances.",
            "Die Türen, die Zugänge sind.",
        ),
    ),
    objects(
        "opening_selector",
        &en_de(
            "The openings that are entrances.",
            "Die Öffnungen, die Zugänge sind.",
        ),
    ),
    objects(
        "space_selector",
        &en_de(
            "The spaces the access path may reach; every object without it.",
            "Die Räume, die der Zugangsweg erreichen darf; ohne Angabe jedes Objekt.",
        ),
    ),
];

/// Whether a shape fits on a space's free floor.
pub(super) const FREE_FLOOR_FIT: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "free_floor_fit",
        parameters: &FREE_FLOOR,
        dimension: None,
        services: &["free-space", "object-frame", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "the free-space service is not registered or cannot search the floor",
            "the spaces searched with the space cannot be read",
        ],
        label: &en_de("Free-floor fit", "Passt auf die freie Bodenfläche"),
        help: &en_de(
            "One item: whether the shape fits on the space's free floor, searched as \
             `free-floor-circle` and `free-floor-rectangle` search it, obstacles and swings \
             the selections cannot decide sent first and a proof of absence asked again \
             without them; undecided where the selections leave it open.",
            "Ein Element: ob die Form auf die freie Bodenfläche des Raums passt, gesucht wie \
             `free-floor-circle` und `free-floor-rectangle` suchen, von den Auswahlen \
             unentschiedene Hindernisse und Aufschläge zuerst mitgeschickt und ein Nachweis \
             des Fehlens ohne sie erneut erfragt; unentschieden, wo die Auswahlen es \
             offenlassen.",
        ),
    },
    fields: &[
        field(
            "fits",
            TRUTH,
            &en_de("Fits", "Passt"),
            &en_de(
                "Whether a placement is found (true) or proven absent (false); undecided \
                 where only what the selections cannot decide stands in its way.",
                "Ob eine Platzierung gefunden (wahr) oder ihr Fehlen nachgewiesen ist \
                 (falsch); unentschieden, wo nur steht, was die Auswahlen nicht entscheiden.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Related", "Bezogen"),
            &en_de(
                "The spaces searched with the space and its entrances.",
                "Die mit dem Raum durchsuchten Räume und seine Zugänge.",
            ),
        ),
        field(
            "unreached",
            TRUTH,
            &en_de("Unreached", "Unerreicht"),
            &en_de(
                "Whether the shape, proven not to be reached from an entrance, fits where no \
                 path reaches it.",
                "Ob die Form, die nachweislich von keinem Zugang erreicht wird, dort passt, \
                 wo kein Weg sie erreicht.",
            ),
        ),
    ],
};

const TEXT: MemberFieldKind = MemberFieldKind::Text;
const LENGTH: MemberFieldKind = MemberFieldKind::Number {
    dimension: Some(crate::QuantityDimension::Length),
};

const fn number(
    key: &'static str,
    minimum: f64,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Number { minimum },
        required: false,
        default: None,
        help,
    }
}

const fn words(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help,
    }
}

/// The connectors a walk may climb, as `escape-route`, `space-distance`
/// and `accessible-route` declare them.
const STAIRS: MeasuredParameter = objects(
    "stair_selector",
    &en_de(
        "The stairs a walk may climb.",
        "Die Treppen, die ein Weg steigen darf.",
    ),
);
const RAMPS: MeasuredParameter = objects(
    "ramp_selector",
    &en_de(
        "The ramps a walk may climb.",
        "Die Rampen, die ein Weg steigen darf.",
    ),
);
const LIFTS: MeasuredParameter = objects(
    "lift_selector",
    &en_de(
        "The lifts a walk may ride.",
        "Die Aufzüge, die ein Weg nutzen darf.",
    ),
);
const STAIR_LENGTH: MeasuredParameter = words(
    "stair_length",
    &en_de(
        "How a climb counts: `slope` (the default) or `horizontal-plus-vertical`.",
        "Wie ein Aufstieg zählt: `slope` (Standard) oder `horizontal-plus-vertical`.",
    ),
);
const VERTICAL_FACTOR: MeasuredParameter = number(
    "vertical_factor",
    0.0,
    &en_de(
        "What a metre of rise counts (1 by default).",
        "Was ein Meter Steigung zählt (standardmäßig 1).",
    ),
);

const DISTANCES: [MeasuredParameter; 16] = [
    MeasuredParameter {
        key: "distances",
        kind: MeasuredParameterKind::Table,
        required: true,
        default: None,
        help: &en_de(
            "The rows of `space-distance`: `from` and `to` selectors, `measure`, \
             `same_storey`, `direct_access`, `minimum`, `maximum` and `label`.",
            "Die Zeilen von `space-distance`: Selektoren `from` und `to`, `measure`, \
             `same_storey`, `direct_access`, `minimum`, `maximum` und `label`.",
        ),
    },
    path(
        "storey_path",
        &en_de(
            "The relationship steps a space's storeys are climbed to.",
            "Die Beziehungsschritte zu den Geschossen eines Raums.",
        ),
    ),
    objects("storey_selector", &en_de("The storeys.", "Die Geschosse.")),
    path(
        "access_path",
        &en_de(
            "The relationship steps from a door or opening to the spaces it opens onto.",
            "Die Beziehungsschritte von einer Tür oder Öffnung zu den Räumen, in die sie \
             führt.",
        ),
    ),
    objects(
        "door_selector",
        &en_de(
            "The doors giving direct access.",
            "Die Türen, die direkten Zugang geben.",
        ),
    ),
    objects(
        "opening_selector",
        &en_de(
            "The openings giving direct access.",
            "Die Öffnungen, die direkten Zugang geben.",
        ),
    ),
    objects(
        "space_selector",
        &en_de(
            "The spaces the access path may reach; every object without it.",
            "Die Räume, die der Zugangsweg erreichen darf; ohne Angabe jedes Objekt.",
        ),
    ),
    length(
        "walking_radius",
        false,
        &en_de(
            "The walking body's radius.",
            "Der Radius des gehenden Körpers.",
        ),
    ),
    length(
        "walking_height",
        false,
        &en_de(
            "The walking body's height.",
            "Die Höhe des gehenden Körpers.",
        ),
    ),
    length(
        "walking_step",
        false,
        &en_de(
            "The step the walking body takes over.",
            "Die Stufe, die der gehende Körper überwindet.",
        ),
    ),
    number(
        "walking_slope",
        f64::NEG_INFINITY,
        &en_de(
            "The slope the walking body climbs; level by default.",
            "Die Neigung, die der gehende Körper steigt; standardmäßig eben.",
        ),
    ),
    STAIRS,
    RAMPS,
    LIFTS,
    STAIR_LENGTH,
    VERTICAL_FACTOR,
];

/// The nearest destination of each row of `space-distance` that applies to
/// a space.
pub(super) const DISTANCE_ROWS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "distance_rows",
        parameters: &DISTANCES,
        dimension: None,
        services: &[
            "plan-span",
            "proximity",
            "metric-routing",
            "vertical-extent",
            "relationship-selection",
        ],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "whether a row's `from` picks the space is undecided",
            "a row's destinations cannot be listed",
        ],
        label: &en_de("Nearest destinations", "Nächste Ziele"),
        help: &en_de(
            "One item per row of `space-distance` whose `from` picks the space: the nearest \
             destination's distance, bounded by every destination that might qualify from \
             below and by the sure ones from above, as the capability measures it, with the \
             row's bounds.",
            "Ein Element je Zeile von `space-distance`, deren `from` den Raum wählt: der \
             Abstand zum nächsten Ziel, von unten durch jedes Ziel begrenzt, das in Frage \
             kommen kann, von oben durch die sicheren, wie die Fähigkeit ihn misst, mit den \
             Grenzen der Zeile.",
        ),
    },
    fields: &[
        field(
            "row",
            TEXT,
            &en_de("Row", "Zeile"),
            &en_de(
                "The row, as findings name it.",
                "Die Zeile, wie Befunde sie nennen.",
            ),
        ),
        field(
            "nearest",
            LENGTH,
            &en_de("Nearest distance", "Nächster Abstand"),
            &en_de(
                "From every destination that might qualify to the sure ones; infinite where \
                 none bounds it; undecided where the destinations cannot be listed.",
                "Von jedem Ziel, das in Frage kommen kann, bis zu den sicheren; unendlich, wo \
                 keines ihn begrenzt; unentschieden, wo die Ziele nicht aufzählbar sind.",
            ),
        ),
        field(
            "minimum",
            LENGTH,
            &en_de("Minimum", "Minimum"),
            &en_de(
                "The row's minimum; `null` without one.",
                "Das Minimum der Zeile; `null` ohne.",
            ),
        ),
        field(
            "maximum",
            LENGTH,
            &en_de("Maximum", "Maximum"),
            &en_de(
                "The row's maximum; `null` without one.",
                "Das Maximum der Zeile; `null` ohne.",
            ),
        ),
        field(
            "above_words",
            TEXT,
            &en_de("Beyond the maximum", "Jenseits des Maximums"),
            &en_de(
                "How lying beyond the maximum is worded: the nearest possible destination, \
                 or none.",
                "Wie das Überschreiten des Maximums formuliert wird: das nächste mögliche Ziel \
                 oder keines.",
            ),
        ),
        field(
            "above_related",
            OBJECTS,
            &en_de("Destinations beyond", "Ziele jenseits"),
            &en_de(
                "The destinations lying beyond the maximum relates.",
                "Die Ziele, auf die sich das Überschreiten bezieht.",
            ),
        ),
        field(
            "below_words",
            TEXT,
            &en_de("Within the minimum", "Unter dem Minimum"),
            &en_de(
                "How lying within the minimum is worded: the nearest sure destination.",
                "Wie das Unterschreiten des Minimums formuliert wird: das nächste sichere Ziel.",
            ),
        ),
        field(
            "below_related",
            OBJECTS,
            &en_de("Destination within", "Ziel darunter"),
            &en_de(
                "The sure destination lying within the minimum.",
                "Das sichere Ziel unter dem Minimum.",
            ),
        ),
        field(
            "open_words",
            TEXT,
            &en_de("Undecided", "Unentschieden"),
            &en_de(
                "Why the distance is not known better, for a row its bounds leave undecided.",
                "Warum der Abstand nicht genauer bekannt ist, für eine Zeile, die ihre Grenzen \
                 offenlassen.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided distance is open, as a report writes it.",
                "Warum ein unentschiedener Abstand offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};

const fn property(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help,
    }
}

const fn truth(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help,
    }
}

/// The traversal a rule reaches members along, each parameter under its
/// own name.
const RELATIONSHIP: MeasuredParameter = words(
    "relationship",
    &en_de(
        "The relationship the members are reached along.",
        "Die Beziehung, über die die Mitglieder erreicht werden.",
    ),
);
const DIRECTION: MeasuredParameter = words(
    "direction",
    &en_de(
        "The relationship's direction: `forward`, `backward` or `either`.",
        "Die Richtung der Beziehung: `forward`, `backward` oder `either`.",
    ),
);
const FOLLOW_CHAIN: MeasuredParameter = truth(
    "follow_chain",
    &en_de(
        "Whether the relationship is followed as a chain.",
        "Ob der Beziehung als Kette gefolgt wird.",
    ),
);
const TRAVERSAL_PATH: MeasuredParameter = path(
    "path",
    &en_de(
        "The relationship steps the members are reached along.",
        "Die Beziehungsschritte, über die die Mitglieder erreicht werden.",
    ),
);
const SKIP_ABSENT: MeasuredParameter = truth(
    "skip_absent_relationship_ends",
    &en_de(
        "Whether relationship ends the source does not hold are skipped.",
        "Ob Beziehungsenden, die die Quelle nicht enthält, übergangen werden.",
    ),
);
const SELECTION: MeasuredParameter = objects(
    "selection",
    &en_de(
        "The objects the rule selects, those it cannot decide possible members.",
        "Die Objekte, die die Regel wählt, die unentschiedenen mögliche Mitglieder.",
    ),
);
const KEY_1: MeasuredParameter = property(
    "key_1",
    &en_de(
        "The property a row's `key_1` tests.",
        "Die Eigenschaft, die `key_1` einer Zeile prüft.",
    ),
);
const KEY_2: MeasuredParameter = property(
    "key_2",
    &en_de(
        "The property a row's `key_2` tests.",
        "Die Eigenschaft, die `key_2` einer Zeile prüft.",
    ),
);
const KEY_3: MeasuredParameter = property(
    "key_3",
    &en_de(
        "The property a row's `key_3` tests.",
        "Die Eigenschaft, die `key_3` einer Zeile prüft.",
    ),
);
const CASE_SENSITIVE: MeasuredParameter = truth(
    "case_sensitive",
    &en_de(
        "Whether key patterns compare case-sensitively (the default).",
        "Ob Schlüsselmuster Groß- und Kleinschreibung unterscheiden (Standard).",
    ),
);

const ALLOCATION: [MeasuredParameter; 18] = [
    MeasuredParameter {
        key: "rows",
        kind: MeasuredParameterKind::Table,
        required: true,
        default: None,
        help: &en_de(
            "The rows of `table-allocation`: key and anchor patterns, `label`, `count`, \
             `area` and its tolerance.",
            "Die Zeilen von `table-allocation`: Schlüssel- und Ankermuster, `label`, \
             `count`, `area` und ihre Toleranz.",
        ),
    },
    words(
        "mode",
        &en_de(
            "How a row is chosen: `first` (the default) or `most_specific`.",
            "Wie eine Zeile gewählt wird: `first` (Standard) oder `most_specific`.",
        ),
    ),
    KEY_1,
    KEY_2,
    KEY_3,
    property(
        "key_4",
        &en_de(
            "The property a row's `key_4` tests.",
            "Die Eigenschaft, die `key_4` einer Zeile prüft.",
        ),
    ),
    CASE_SENSITIVE,
    property(
        "area_property",
        &en_de(
            "The area quantity an object states, in place of its measured footprint.",
            "Die Flächenangabe eines Objekts anstelle seiner gemessenen Grundfläche.",
        ),
    ),
    words(
        "area_mode",
        &en_de(
            "`sum` (the default): a row's objects' areas summed; `each`: each object's own \
             area a match condition.",
            "`sum` (Standard): die Flächen der Objekte einer Zeile summiert; `each`: die \
             eigene Fläche jedes Objekts als Bedingung.",
        ),
    ),
    objects(
        "anchor_selector",
        &en_de(
            "The anchors whose reached objects are judged together.",
            "Die Anker, deren erreichte Objekte zusammen beurteilt werden.",
        ),
    ),
    property(
        "anchor_key",
        &en_de(
            "The anchor's property a row's `anchor` pattern tests.",
            "Die Eigenschaft des Ankers, die das Muster `anchor` einer Zeile prüft.",
        ),
    ),
    truth(
        "across_sources",
        &en_de(
            "Whether the whole project is one group, not each source.",
            "Ob das ganze Projekt eine Gruppe ist statt jeder Quelle.",
        ),
    ),
    RELATIONSHIP,
    DIRECTION,
    FOLLOW_CHAIN,
    TRAVERSAL_PATH,
    SKIP_ABSENT,
    SELECTION,
];

/// The allocation of `table-allocation`, of the project.
pub(super) const ALLOCATIONS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "allocations",
        parameters: &ALLOCATION,
        dimension: None,
        services: &["property-resolution", "relationship-selection", "plan-area"],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Project,
        not_evaluated: &[
            "the project has no source to allocate in",
            "an object's row, a group's rows or a row's area cannot be told",
        ],
        label: &en_de("Allocation to rows", "Zuordnung zu Zeilen"),
        help: &en_de(
            "The objects the rule selects, each assigned to one row as `table-allocation` \
             assigns them: an item per object no row matches or left open, per group whose \
             rows cannot be told, and per row of a group, its objects counted and their \
             summed area, each naming where its outcome goes.",
            "Die Objekte, die die Regel wählt, jedes einer Zeile zugeordnet wie \
             `table-allocation` sie zuordnet: ein Element je Objekt, das keine Zeile trifft \
             oder offen bleibt, je Gruppe, deren Zeilen sich nicht bestimmen lassen, und je \
             Zeile einer Gruppe, ihre Objekte gezählt und ihre Flächen summiert, jedes mit \
             dem Ort seines Ergebnisses.",
        ),
    },
    fields: &[
        field(
            "at",
            OBJECTS,
            &en_de("Where", "Wo"),
            &en_de(
                "The object, anchor, or source's or project's stand-in the item's outcome \
                 goes to.",
                "Das Objekt, der Anker oder der Platzhalter der Quelle oder des Projekts, zu \
                 dem das Ergebnis des Elements gehört.",
            ),
        ),
        field(
            "extra",
            TRUTH,
            &en_de("Extra", "Überzählig"),
            &en_de(
                "An object no row matches.",
                "Ein Objekt, das keine Zeile trifft.",
            ),
        ),
        field(
            "keys",
            TEXT,
            &en_de("Keys", "Schlüssel"),
            &en_de(
                "The extra's key values.",
                "Die Schlüsselwerte des überzähligen Objekts.",
            ),
        ),
        field(
            "open",
            TRUTH,
            &en_de("Open", "Offen"),
            &en_de(
                "Undecided, with why: an object's row or anchorship, or a group's rows.",
                "Unentschieden, mit Grund: die Zeile oder Ankerschaft eines Objekts oder die \
                 Zeilen einer Gruppe.",
            ),
        ),
        field(
            "name",
            TEXT,
            &en_de("Row", "Zeile"),
            &en_de(
                "The row, as findings name it.",
                "Die Zeile, wie Befunde sie nennen.",
            ),
        ),
        field(
            "place",
            TEXT,
            &en_de("Place", "Ort"),
            &en_de(
                "Where the group lies, as findings word it.",
                "Wo die Gruppe liegt, wie Befunde es formulieren.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Assigned", "Zugeordnet"),
            &en_de(
                "The objects surely assigned to the row in the group.",
                "Die der Zeile in der Gruppe sicher zugeordneten Objekte.",
            ),
        ),
        field(
            "found",
            RATIO,
            &en_de("Objects", "Objekte"),
            &en_de(
                "From the objects surely assigned to every one that may belong.",
                "Von den sicher zugeordneten Objekten bis zu jedem, das dazugehören kann.",
            ),
        ),
        field(
            "count",
            RATIO,
            &en_de("Required count", "Geforderte Anzahl"),
            &en_de(
                "The row's `count`; `null` without one.",
                "Die `count` der Zeile; `null` ohne.",
            ),
        ),
        field(
            "empty",
            TRUTH,
            &en_de("Empty", "Leer"),
            &en_de(
                "Whether the row matched nothing and nothing may still belong to it.",
                "Ob die Zeile nichts traf und nichts mehr dazugehören kann.",
            ),
        ),
        field(
            "open_count",
            RATIO,
            &en_de("May belong", "Kann dazugehören"),
            &en_de(
                "How many more objects may belong to the row.",
                "Wie viele weitere Objekte zur Zeile gehören können.",
            ),
        ),
        field(
            "sum",
            AREA,
            &en_de("Summed area", "Summierte Fläche"),
            &en_de(
                "The assigned objects' summed area; undecided where one cannot be measured.",
                "Die summierte Fläche der zugeordneten Objekte; unentschieden, wo eine nicht \
                 messbar ist.",
            ),
        ),
        field(
            "low",
            AREA,
            &en_de("Least area", "Kleinste Fläche"),
            &en_de(
                "The row's area less its tolerance.",
                "Die Fläche der Zeile abzüglich Toleranz.",
            ),
        ),
        field(
            "high",
            AREA,
            &en_de("Greatest area", "Größte Fläche"),
            &en_de(
                "The row's area plus its tolerance.",
                "Die Fläche der Zeile zuzüglich Toleranz.",
            ),
        ),
        field(
            "required",
            TEXT,
            &en_de("Required area", "Geforderte Fläche"),
            &en_de(
                "The row's area as findings word it.",
                "Die Fläche der Zeile, wie Befunde sie nennen.",
            ),
        ),
        field(
            "more",
            TRUTH,
            &en_de("More may belong", "Weitere möglich"),
            &en_de(
                "Whether objects that may still belong to the row could add area.",
                "Ob Objekte, die noch zur Zeile gehören können, Fläche hinzufügen könnten.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided item is open, as a report writes it.",
                "Warum ein unentschiedenes Element offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};

const COMPOSITION: [MeasuredParameter; 18] = [
    MeasuredParameter {
        key: "requirements",
        kind: MeasuredParameterKind::Table,
        required: true,
        default: None,
        help: &en_de(
            "The member entries of `group-composition`: key and group patterns, `label` and \
             `count`.",
            "Die Mitgliedereinträge von `group-composition`: Schlüssel- und Gruppenmuster, \
             `label` und `count`.",
        ),
    },
    KEY_1,
    KEY_2,
    KEY_3,
    CASE_SENSITIVE,
    property(
        "group_key",
        &en_de(
            "The group's property a row's `group` pattern tests (the older name of \
             `group_key_1`).",
            "Die Eigenschaft der Gruppe, die das Muster `group` einer Zeile prüft (der ältere \
             Name von `group_key_1`).",
        ),
    ),
    property(
        "group_key_1",
        &en_de(
            "The group's property a row's `group` pattern tests.",
            "Die Eigenschaft der Gruppe, die das Muster `group` einer Zeile prüft.",
        ),
    ),
    property(
        "group_key_2",
        &en_de(
            "The group's property a row's `group_2` pattern tests.",
            "Die Eigenschaft der Gruppe, die das Muster `group_2` einer Zeile prüft.",
        ),
    ),
    property(
        "group_key_3",
        &en_de(
            "The group's property a row's `group_3` pattern tests.",
            "Die Eigenschaft der Gruppe, die das Muster `group_3` einer Zeile prüft.",
        ),
    ),
    truth(
        "report_absent_groups",
        &en_de(
            "Whether a row no group in the model matches is found.",
            "Ob eine Zeile, die keine Gruppe im Modell trifft, gemeldet wird.",
        ),
    ),
    objects(
        "member_selector",
        &en_de(
            "The reached objects that are members; every one without it.",
            "Die erreichten Objekte, die Mitglieder sind; ohne Angabe jedes.",
        ),
    ),
    objects(
        "ungrouped_selector",
        &en_de(
            "The objects some selected group must reach.",
            "Die Objekte, die eine gewählte Gruppe erreichen muss.",
        ),
    ),
    RELATIONSHIP,
    DIRECTION,
    FOLLOW_CHAIN,
    TRAVERSAL_PATH,
    SKIP_ABSENT,
    SELECTION,
];

/// The matching of `group-composition`, of the project.
pub(super) const COMPOSITIONS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "compositions",
        parameters: &COMPOSITION,
        dimension: None,
        services: &["property-resolution", "relationship-selection"],
        exactness: MeasuredExactness::Stated,
        subject: MeasuredSubject::Project,
        not_evaluated: &[
            "a group cannot be walked, or its requirements, members or their keys cannot be \
             told",
        ],
        label: &en_de("Group compositions", "Gruppenzusammensetzungen"),
        help: &en_de(
            "Each selected group's members matched to the requirement entries by a maximum \
             matching, as `group-composition` matches them: an item per part every maximum \
             matching leaves short or with a surplus, per group no row matches, per row no \
             group matches, per object no group reaches and per group or object left open, \
             each naming where its outcome goes.",
            "Die Mitglieder jeder gewählten Gruppe den Anforderungseinträgen durch eine \
             größte Zuordnung zugeordnet, wie `group-composition` sie zuordnet: ein Element \
             je Teil, den jede größte Zuordnung unterbesetzt oder mit Überschuss lässt, je \
             Gruppe ohne passende Zeile, je Zeile ohne passende Gruppe, je Objekt, das keine \
             Gruppe erreicht, und je offen bleibender Gruppe oder Objekt, jedes mit dem Ort \
             seines Ergebnisses.",
        ),
    },
    fields: &[
        field(
            "at",
            OBJECTS,
            &en_de("Where", "Wo"),
            &en_de(
                "The group, member, object or the project's stand-in the outcome goes to.",
                "Die Gruppe, das Mitglied, das Objekt oder der Platzhalter des Projekts, zu \
                 dem das Ergebnis gehört.",
            ),
        ),
        field(
            "open",
            TRUTH,
            &en_de("Open", "Offen"),
            &en_de("Undecided, with why.", "Unentschieden, mit Grund."),
        ),
        field(
            "unmatched",
            TRUTH,
            &en_de("No row", "Keine Zeile"),
            &en_de(
                "A group no row with a group cell matches.",
                "Eine Gruppe, die keine Zeile mit Gruppenzelle trifft.",
            ),
        ),
        field(
            "absent",
            TRUTH,
            &en_de("Not in model", "Nicht im Modell"),
            &en_de(
                "A row no group in the model matches.",
                "Eine Zeile, die keine Gruppe im Modell trifft.",
            ),
        ),
        field(
            "ungrouped",
            TRUTH,
            &en_de("In no group", "In keiner Gruppe"),
            &en_de(
                "An object no selected group reaches.",
                "Ein Objekt, das keine gewählte Gruppe erreicht.",
            ),
        ),
        field(
            "keys",
            TEXT,
            &en_de("Keys", "Schlüssel"),
            &en_de(
                "The key values of a group no row matches or a member no entry fits.",
                "Die Schlüsselwerte einer Gruppe ohne passende Zeile oder eines Mitglieds \
                 ohne passenden Eintrag.",
            ),
        ),
        field(
            "name",
            TEXT,
            &en_de("Row", "Zeile"),
            &en_de(
                "The row no group matches.",
                "Die Zeile, die keine Gruppe trifft.",
            ),
        ),
        field(
            "subject",
            TEXT,
            &en_de("Entries", "Einträge"),
            &en_de(
                "The entries left short, with their verb.",
                "Die unterbesetzten Einträge, mit ihrem Verb.",
            ),
        ),
        field(
            "filled",
            RATIO,
            &en_de("Members", "Mitglieder"),
            &en_de(
                "How many members the entries left short hold.",
                "Wie viele Mitglieder die unterbesetzten Einträge haben.",
            ),
        ),
        field(
            "places",
            RATIO,
            &en_de("Places", "Plätze"),
            &en_de(
                "How many members the entries take.",
                "Wie viele Mitglieder die Einträge aufnehmen.",
            ),
        ),
        field(
            "missing",
            RATIO,
            &en_de("Missing", "Fehlend"),
            &en_de(
                "The places less the members.",
                "Die Plätze abzüglich der Mitglieder.",
            ),
        ),
        field(
            "found",
            RATIO,
            &en_de("Fitting members", "Passende Mitglieder"),
            &en_de(
                "How many members compete for the entries.",
                "Wie viele Mitglieder um die Einträge konkurrieren.",
            ),
        ),
        field(
            "surplus",
            RATIO,
            &en_de("Surplus", "Überschuss"),
            &en_de(
                "The members less the places.",
                "Die Mitglieder abzüglich der Plätze.",
            ),
        ),
        field(
            "unfit",
            TRUTH,
            &en_de("Fits no entry", "Passt zu keinem Eintrag"),
            &en_de(
                "Whether the surplus member fits no entry at all.",
                "Ob das überzählige Mitglied zu gar keinem Eintrag passt.",
            ),
        ),
        field(
            "entries",
            TEXT,
            &en_de("Competed entries", "Umkämpfte Einträge"),
            &en_de(
                "The entries a surplus competes for.",
                "Die Einträge, um die ein Überschuss konkurriert.",
            ),
        ),
        field(
            "verb",
            TEXT,
            &en_de("Verb", "Verb"),
            &en_de("`takes` or `take`.", "`takes` oder `take`."),
        ),
        field(
            "via",
            TEXT,
            &en_de("Relation", "Beziehung"),
            &en_de(
                "How a group reaches its members, as findings word it.",
                "Wie eine Gruppe ihre Mitglieder erreicht, wie Befunde es formulieren.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Members concerned", "Betroffene Mitglieder"),
            &en_de(
                "The members a finding relates.",
                "Die Mitglieder, auf die sich ein Befund bezieht.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided item is open, as a report writes it.",
                "Warum ein unentschiedenes Element offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};

const CLEARANCE_CHECKS_PARAMETERS: [MeasuredParameter; 46] = [
    MeasuredParameter {
        key: "side",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `side`, as `component-clearance` reads it.",
            "Der Parameter `side` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "sides",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `sides`, as `component-clearance` reads it.",
            "Der Parameter `sides` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "quantifier",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `quantifier`, as `component-clearance` reads it.",
            "Der Parameter `quantifier` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "front_axis",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `front_axis`, as `component-clearance` reads it.",
            "Der Parameter `front_axis` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "both_sides",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `both_sides`, as `component-clearance` reads it.",
            "Der Parameter `both_sides` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width`, as `component-clearance` reads it.",
            "Der Parameter `width` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_mode",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_mode`, as `component-clearance` reads it.",
            "Der Parameter `width_mode` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_minimum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_minimum`, as `component-clearance` reads it.",
            "Der Parameter `width_minimum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_maximum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_maximum`, as `component-clearance` reads it.",
            "Der Parameter `width_maximum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth`, as `component-clearance` reads it.",
            "Der Parameter `depth` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_mode",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_mode`, as `component-clearance` reads it.",
            "Der Parameter `depth_mode` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_minimum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_minimum`, as `component-clearance` reads it.",
            "Der Parameter `depth_minimum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_maximum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_maximum`, as `component-clearance` reads it.",
            "Der Parameter `depth_maximum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_from",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_from`, as `component-clearance` reads it.",
            "Der Parameter `depth_from` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "radius",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `radius`, as `component-clearance` reads it.",
            "Der Parameter `radius` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "height",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `height`, as `component-clearance` reads it.",
            "Der Parameter `height` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "height_mode",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `height_mode`, as `component-clearance` reads it.",
            "Der Parameter `height_mode` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "height_minimum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `height_minimum`, as `component-clearance` reads it.",
            "Der Parameter `height_minimum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "height_maximum",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `height_maximum`, as `component-clearance` reads it.",
            "Der Parameter `height_maximum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "size_mode",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `size_mode`, as `component-clearance` reads it.",
            "Der Parameter `size_mode` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "size_tolerance",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `size_tolerance`, as `component-clearance` reads it.",
            "Der Parameter `size_tolerance` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "offset",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `offset`, as `component-clearance` reads it.",
            "Der Parameter `offset` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "lateral_offset",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `lateral_offset`, as `component-clearance` reads it.",
            "Der Parameter `lateral_offset` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "align",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `align`, as `component-clearance` reads it.",
            "Der Parameter `align` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "slide_from",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `slide_from`, as `component-clearance` reads it.",
            "Der Parameter `slide_from` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "slide_to",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `slide_to`, as `component-clearance` reads it.",
            "Der Parameter `slide_to` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_slide_from",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_slide_from`, as `component-clearance` reads it.",
            "Der Parameter `depth_slide_from` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "depth_slide_to",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `depth_slide_to`, as `component-clearance` reads it.",
            "Der Parameter `depth_slide_to` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "height_reference",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `height_reference`, as `component-clearance` reads it.",
            "Der Parameter `height_reference` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "vertical_offset",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `vertical_offset`, as `component-clearance` reads it.",
            "Der Parameter `vertical_offset` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "top_datum",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `top_datum`, as `component-clearance` reads it.",
            "Der Parameter `top_datum` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "top_offset",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `top_offset`, as `component-clearance` reads it.",
            "Der Parameter `top_offset` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "obstacles",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `obstacles`, as `component-clearance` reads it.",
            "Der Parameter `obstacles` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "allowed_intruders",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `allowed_intruders`, as `component-clearance` reads it.",
            "Der Parameter `allowed_intruders` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "protrusion",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `protrusion`, as `component-clearance` reads it.",
            "Der Parameter `protrusion` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "within_space",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `within_space`, as `component-clearance` reads it.",
            "Der Parameter `within_space` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "space_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `space_path`, as `component-clearance` reads it.",
            "Der Parameter `space_path` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "wall_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `wall_selector`, as `component-clearance` reads it.",
            "Der Parameter `wall_selector` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "wall_reach",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `wall_reach`, as `component-clearance` reads it.",
            "Der Parameter `wall_reach` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "wall_inset",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `wall_inset`, as `component-clearance` reads it.",
            "Der Parameter `wall_inset` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "support_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `support_selector`, as `component-clearance` reads it.",
            "Der Parameter `support_selector` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "support_tolerance",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `support_tolerance`, as `component-clearance` reads it.",
            "Der Parameter `support_tolerance` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_width_property",
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_width_property`, as `component-clearance` reads it.",
            "Der Parameter `clear_width_property` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_width_from_leaves",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_width_from_leaves`, as `component-clearance` reads it.",
            "Der Parameter `clear_width_from_leaves` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "overall_width",
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `overall_width`, as `component-clearance` reads it.",
            "Der Parameter `overall_width` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_deduction",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_deduction`, as `component-clearance` reads it.",
            "Der Parameter `width_deduction` der Regel, wie `component-clearance` ihn liest.",
        ),
    },
];

pub(super) const CLEARANCE_CHECKS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "clearance_checks",
        parameters: &CLEARANCE_CHECKS_PARAMETERS,
        dimension: None,
        services: &[
            "free-space",
            "object-frame",
            "vertical-extent",
            "plan-span",
            "relationship-selection",
        ],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "the component's frame, extents or spaces cannot be read, or a service is not registered",
        ],
        label: &en_de("Clearance checks", "Freiraumprüfungen"),
        help: &en_de(
            "Each question component-clearance asks of a side of the component, as its search answers it: the volume free less its tolerance, each larger volume obstructed, the volume inside its spaces, its base supported (the support coverage); or, with quantifier any, one answer for every side.",
            "Jede Frage, die component-clearance an eine Seite des Bauteils stellt, wie ihre Suche sie beantwortet: das Volumen abzüglich Toleranz frei, jedes größere verstellt, das Volumen in seinen Räumen, seine Basis getragen (die Auflagerdeckung); oder, mit quantifier any, eine Antwort für alle Seiten.",
        ),
    },
    fields: &[
        field(
            "label",
            TEXT,
            &en_de("Side", "Seite"),
            &en_de(
                "The side or sides the answer is about, as findings name them.",
                "Die Seite oder Seiten, um die es geht, wie Befunde sie nennen.",
            ),
        ),
        field(
            "met",
            TRUTH,
            &en_de("Met", "Erfüllt"),
            &en_de(
                "Whether the question is met: the volume free, a larger one obstructed, inside its spaces, supported; undecided, with why, where the positions or selections leave it open.",
                "Ob die Frage erfüllt ist: das Volumen frei, ein größeres verstellt, innerhalb seiner Räume, getragen; unentschieden, mit Grund, wo Lagen oder Auswahlen es offenlassen.",
            ),
        ),
        field(
            "words",
            TEXT,
            &en_de("Words", "Worte"),
            &en_de(
                "What an unmet question comes to, as findings word it after the side.",
                "Was eine nicht erfüllte Frage ergibt, wie Befunde es nach der Seite formulieren.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Related", "Bezogen"),
            &en_de(
                "The obstacles, or the spaces the volume leaves.",
                "Die Hindernisse oder die Räume, die das Volumen verlässt.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided answer is open, as a report writes it.",
                "Warum eine unentschiedene Antwort offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};

const ROUTE_VERDICTS_PARAMETERS: [MeasuredParameter; 22] = [
    MeasuredParameter {
        key: "route_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `route_selector`, as `accessible-route` reads it.",
            "Der Parameter `route_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "start_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `start_selector`, as `accessible-route` reads it.",
            "Der Parameter `start_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "portal_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `portal_selector`, as `accessible-route` reads it.",
            "Der Parameter `portal_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "lift_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `lift_selector`, as `accessible-route` reads it.",
            "Der Parameter `lift_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "ramp_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `ramp_selector`, as `accessible-route` reads it.",
            "Der Parameter `ramp_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "stair_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `stair_selector`, as `accessible-route` reads it.",
            "Der Parameter `stair_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "obstacle_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `obstacle_selector`, as `accessible-route` reads it.",
            "Der Parameter `obstacle_selector` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "subtract_door_swings",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `subtract_door_swings`, as `accessible-route` reads it.",
            "Der Parameter `subtract_door_swings` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_metres`, as `accessible-route` reads it.",
            "Der Parameter `width_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_height_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_height_metres`, as `accessible-route` reads it.",
            "Der Parameter `clear_height_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "door_width_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `door_width_metres`, as `accessible-route` reads it.",
            "Der Parameter `door_width_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "ramp_width_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `ramp_width_metres`, as `accessible-route` reads it.",
            "Der Parameter `ramp_width_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "stair_width_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `stair_width_metres`, as `accessible-route` reads it.",
            "Der Parameter `stair_width_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "forbid_stairs",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `forbid_stairs`, as `accessible-route` reads it.",
            "Der Parameter `forbid_stairs` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_width_property",
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_width_property`, as `accessible-route` reads it.",
            "Der Parameter `clear_width_property` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "obstruction_depth_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `obstruction_depth_metres`, as `accessible-route` reads it.",
            "Der Parameter `obstruction_depth_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "surface_gap_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `surface_gap_metres`, as `accessible-route` reads it.",
            "Der Parameter `surface_gap_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_width_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_width_metres`, as `accessible-route` reads it.",
            "Der Parameter `passing_width_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_length_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_length_metres`, as `accessible-route` reads it.",
            "Der Parameter `passing_length_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_spacing_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_spacing_metres`, as `accessible-route` reads it.",
            "Der Parameter `passing_spacing_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_reach_metres",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_reach_metres`, as `accessible-route` reads it.",
            "Der Parameter `passing_reach_metres` der Regel, wie `accessible-route` ihn liest.",
        ),
    },
    SELECTION,
];

pub(super) const ROUTE_VERDICTS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "route_verdicts",
        parameters: &ROUTE_VERDICTS_PARAMETERS,
        dimension: None,
        services: &[
            "walkability",
            "walking-surface",
            "metric-routing",
            "free-space",
            "plan-span",
            "vertical-extent",
        ],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "the walkability service is not registered or takes no snapshot, or the scene cannot be selected",
        ],
        label: &en_de("Route verdicts", "Wegurteile"),
        help: &en_de(
            "One item: whether some start reaches the destination for the rule's body through the route spaces, with its passing spaces, as accessible-route walks it, and what blocks it where it is cut off.",
            "Ein Element: ob ein Start das Ziel für den Körper der Regel durch die Wegräume erreicht, mit seinen Ausweichstellen, wie accessible-route geht, und was es blockiert, wo es abgeschnitten ist.",
        ),
    },
    fields: &[
        field(
            "reached",
            TRUTH,
            &en_de("Reached", "Erreicht"),
            &en_de(
                "Whether some start reaches the destination for the rule's body, with its passing spaces; undecided, with why, where the walk cannot tell.",
                "Ob ein Start das Ziel für den Körper der Regel erreicht, mit seinen Ausweichstellen; unentschieden, mit Grund, wo der Weg es nicht entscheidet.",
            ),
        ),
        field(
            "words",
            TEXT,
            &en_de("Words", "Worte"),
            &en_de(
                "What cuts the destination off, or which route lacks its passing spaces, as findings word it.",
                "Was das Ziel abschneidet oder welchem Weg seine Ausweichstellen fehlen, wie Befunde es formulieren.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Related", "Bezogen"),
            &en_de(
                "The elements that block it, or the starts.",
                "Die Elemente, die es blockieren, oder die Starts.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided answer is open, as a report writes it.",
                "Warum eine unentschiedene Antwort offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};

const CIRCULATION_VERDICTS_PARAMETERS: [MeasuredParameter; 33] = [
    MeasuredParameter {
        key: "component_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `component_selector`, as `local-circulation` reads it.",
            "Der Parameter `component_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "space_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `space_path`, as `local-circulation` reads it.",
            "Der Parameter `space_path` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "access_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `access_path`, as `local-circulation` reads it.",
            "Der Parameter `access_path` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "door_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `door_selector`, as `local-circulation` reads it.",
            "Der Parameter `door_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "opening_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `opening_selector`, as `local-circulation` reads it.",
            "Der Parameter `opening_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "space_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `space_selector`, as `local-circulation` reads it.",
            "Der Parameter `space_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "obstacles",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `obstacles`, as `local-circulation` reads it.",
            "Der Parameter `obstacles` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "subtract_door_swings",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `subtract_door_swings`, as `local-circulation` reads it.",
            "Der Parameter `subtract_door_swings` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_metres`, as `local-circulation` reads it.",
            "Der Parameter `width_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_height_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_height_metres`, as `local-circulation` reads it.",
            "Der Parameter `clear_height_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "tolerance_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `tolerance_metres`, as `local-circulation` reads it.",
            "Der Parameter `tolerance_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "component_mode",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `component_mode`, as `local-circulation` reads it.",
            "Der Parameter `component_mode` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "end_width_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `end_width_metres`, as `local-circulation` reads it.",
            "Der Parameter `end_width_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "end_length_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `end_length_metres`, as `local-circulation` reads it.",
            "Der Parameter `end_length_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "end_reach_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `end_reach_metres`, as `local-circulation` reads it.",
            "Der Parameter `end_reach_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "short_end_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `short_end_metres`, as `local-circulation` reads it.",
            "Der Parameter `short_end_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "narrow_end_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `narrow_end_metres`, as `local-circulation` reads it.",
            "Der Parameter `narrow_end_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "merge_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `merge_path`, as `local-circulation` reads it.",
            "Der Parameter `merge_path` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "band_from_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `band_from_metres`, as `local-circulation` reads it.",
            "Der Parameter `band_from_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "end_exempt_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `end_exempt_selector`, as `local-circulation` reads it.",
            "Der Parameter `end_exempt_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "end_exempt_reach_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `end_exempt_reach_metres`, as `local-circulation` reads it.",
            "Der Parameter `end_exempt_reach_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "partner_selector",
        kind: MeasuredParameterKind::Objects,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `partner_selector`, as `local-circulation` reads it.",
            "Der Parameter `partner_selector` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "require_entrances",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `require_entrances`, as `local-circulation` reads it.",
            "Der Parameter `require_entrances` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "check_entrance_width",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `check_entrance_width`, as `local-circulation` reads it.",
            "Der Parameter `check_entrance_width` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_width_property",
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_width_property`, as `local-circulation` reads it.",
            "Der Parameter `clear_width_property` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "clear_width_from_leaves",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `clear_width_from_leaves`, as `local-circulation` reads it.",
            "Der Parameter `clear_width_from_leaves` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "overall_width",
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help: &en_de(
            "The rule's `overall_width`, as `local-circulation` reads it.",
            "Der Parameter `overall_width` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "width_deduction",
        kind: MeasuredParameterKind::Length {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `width_deduction`, as `local-circulation` reads it.",
            "Der Parameter `width_deduction` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_width_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_width_metres`, as `local-circulation` reads it.",
            "Der Parameter `passing_width_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_length_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_length_metres`, as `local-circulation` reads it.",
            "Der Parameter `passing_length_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_spacing_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_spacing_metres`, as `local-circulation` reads it.",
            "Der Parameter `passing_spacing_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    MeasuredParameter {
        key: "passing_reach_metres",
        kind: MeasuredParameterKind::Number {
            minimum: f64::NEG_INFINITY,
        },
        required: false,
        default: None,
        help: &en_de(
            "The rule's `passing_reach_metres`, as `local-circulation` reads it.",
            "Der Parameter `passing_reach_metres` der Regel, wie `local-circulation` ihn liest.",
        ),
    },
    SELECTION,
];

pub(super) const CIRCULATION_VERDICTS: MemberDescriptor = MemberDescriptor {
    list: MeasuredDescriptor {
        name: "circulation_verdicts",
        parameters: &CIRCULATION_VERDICTS_PARAMETERS,
        dimension: None,
        services: &[
            "free-space",
            "relationship-selection",
            "object-frame",
            "metric-routing",
            "plan-span",
            "vertical-extent",
            "proximity",
        ],
        exactness: MeasuredExactness::Measured,
        subject: MeasuredSubject::Object,
        not_evaluated: &[
            "the free-space service is not registered or cannot map a space, or a selection cannot be listed",
        ],
        label: &en_de("Circulation verdicts", "Bewegungsflächenurteile"),
        help: &en_de(
            "Every answer local-circulation's search gives about the spaces a rule selects and their components: entrances, reach, links, path ends and passing spaces, each on the object it is about.",
            "Jede Antwort, die die Suche von local-circulation zu den gewählten Räumen und ihren Bauteilen gibt: Zugänge, Erreichbarkeit, Verbindungen, Wegenden und Ausweichstellen, jede beim Objekt, um das es geht.",
        ),
    },
    fields: &[
        field(
            "at",
            OBJECTS,
            &en_de("Where", "Wo"),
            &en_de(
                "The space or component the answer is about.",
                "Der Raum oder das Bauteil, um das es geht.",
            ),
        ),
        field(
            "met",
            TRUTH,
            &en_de("Met", "Erfüllt"),
            &en_de(
                "Whether the answer meets the requirement (false for each the search finds unmet); undecided, with why, where the search cannot tell.",
                "Ob die Antwort die Anforderung erfüllt (falsch für jede, die die Suche als nicht erfüllt findet); unentschieden, mit Grund, wo die Suche es nicht entscheidet.",
            ),
        ),
        field(
            "words",
            TEXT,
            &en_de("Words", "Worte"),
            &en_de(
                "What an unmet answer comes to, as findings word it.",
                "Was eine nicht erfüllte Antwort ergibt, wie Befunde es formulieren.",
            ),
        ),
        field(
            "related",
            OBJECTS,
            &en_de("Related", "Bezogen"),
            &en_de(
                "The objects an unmet answer relates.",
                "Die Objekte, auf die sich eine nicht erfüllte Antwort bezieht.",
            ),
        ),
        field(
            "reason",
            TEXT,
            &en_de("Reason", "Grund"),
            &en_de(
                "Why an undecided answer is open, as a report writes it.",
                "Warum eine unentschiedene Antwort offen ist, wie ein Bericht es schreibt.",
            ),
        ),
    ],
};
