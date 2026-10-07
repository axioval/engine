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
