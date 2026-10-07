//! The member lists the search capabilities' templates judge (#286): each
//! a search kept in built-in code, answering per checked object what it
//! found (a fit, a route, a distance, an allocation), which the template
//! decides over.

use super::super::registry::en_de;
use super::super::{
    LocalizedText, MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind,
    MeasuredSubject,
};
use super::{MemberDescriptor, MemberFieldKind, TRUTH, field};

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
