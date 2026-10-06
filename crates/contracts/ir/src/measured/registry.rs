//! Every measured value's descriptor, sorted by name.

use super::{
    ANGLE_TO, BEARING, CLEAR_HEIGHT, CLEAR_WIDTH, CLEARANCE_BELOW, COUNT_WITHIN, CROSS_FALL,
    DISTANCE, EXTENT, GRADIENT_DIRECTION, HEADROOM, INCLINATION, LENGTH, LocalizedText,
    MeasuredDescriptor, MeasuredExactness, MeasuredParameter, MeasuredParameterKind, PERIMETER,
    SKEW, SLOPE, SLOPE_ALONG, THICKNESS,
};
use crate::{
    MEASURED_AREA, MEASURED_BOTTOM, MEASURED_BOTTOM_ABOVE_LEVEL, MEASURED_BOUNDARY_AREA,
    MEASURED_EXTENT_X, MEASURED_EXTENT_Y, MEASURED_EXTENT_Z, MEASURED_LEVEL_HEIGHT, MEASURED_TOP,
    MEASURED_VOLUME, MEASURED_X, MEASURED_Y, MEASURED_Z, QuantityDimension,
};

/// English and German texts, in that order.
pub const fn en_de(en: &'static str, de: &'static str) -> [LocalizedText; 2] {
    [
        LocalizedText {
            language: "en",
            text: en,
        },
        LocalizedText {
            language: "de",
            text: de,
        },
    ]
}

const VERTICAL: &[&str] = &["vertical-extent"];
const FACES: &[&str] = &["vertical-extent"];
const FACES_AND_FRAMES: &[&str] = &["vertical-extent", "object-frame"];
pub(super) const NO_FACE: &str = "the body has no such face, or it cannot be told apart";
const STANDS_VERTICAL: &str = "a piece of the face stands vertical, so it has no gradient";

/// The face a surface value reads: the pieces looking up or down, or those
/// facing a direction ([`FACING`]).
pub(super) const FACE: MeasuredParameter = MeasuredParameter {
    key: "face",
    kind: MeasuredParameterKind::Choice {
        options: &["top", "bottom", "facing"],
    },
    required: false,
    default: Some("top"),
    help: &en_de(
        "The face read: `top`, looking up, or `bottom`, looking down (an open \
         surface is both), or `facing`, the pieces of a closed body whose outward \
         normal lies within `tolerance` of `direction`.",
        "Die gelesene Fläche: `top`, nach oben, oder `bottom`, nach unten (eine \
         offene Fläche ist beides), oder `facing`, die Teile eines geschlossenen \
         Körpers, deren äußere Normale höchstens `tolerance` von `direction` abweicht.",
    ),
};

/// The face a surface value reads where `direction` is taken: up or down.
const FACE_UP_OR_DOWN: MeasuredParameter = MeasuredParameter {
    key: "face",
    kind: MeasuredParameterKind::Choice {
        options: &["top", "bottom"],
    },
    required: false,
    default: Some("top"),
    help: &en_de(
        "The face read: `top`, looking up, or `bottom`, looking down. An open \
         surface is both.",
        "Die gelesene Fläche: `top`, nach oben, oder `bottom`, nach unten. Eine \
         offene Fläche ist beides.",
    ),
};

/// The direction and tolerance of `face=facing`, stated exactly with it.
pub(super) const FACING: [MeasuredParameter; 2] = [
    MeasuredParameter {
        key: "direction",
        kind: MeasuredParameterKind::Vector,
        required: false,
        default: None,
        help: &en_de(
            "With `face=facing`: the direction the pieces face, `x,y,z` in world \
             coordinates.",
            "Mit `face=facing`: die Richtung, in die die Teile zeigen, `x,y,z` in \
             Weltkoordinaten.",
        ),
    },
    MeasuredParameter {
        key: "tolerance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "With `face=facing`: how far a piece's outward normal may lean from \
             `direction`, in degrees, at most 180.",
            "Mit `face=facing`: wie weit die äußere Normale eines Teils von \
             `direction` abweichen darf, in Grad, höchstens 180.",
        ),
    },
];

/// Why a face's pieces facing a direction cannot be told apart.
pub(super) const FACING_UNDECIDED: &str = "whether a piece faces the direction cannot be decided";

const RECTANGLES: &[&str] = &["plan-span", "relationship-selection"];
const NO_LONG_AXIS: &str = "a footprint has no long axis of its own (a square, a tie, a \
     tessellation)";

const REFERENCE_PATH: MeasuredParameter = MeasuredParameter {
    key: "path",
    kind: MeasuredParameterKind::Path,
    required: true,
    default: None,
    help: &en_de(
        "The relationship steps from the object to the objects it is measured against; \
         the angle is the hull over every one reached, none when none is.",
        "Die Beziehungsschritte vom Objekt zu den Objekten, gegen die gemessen wird; \
         der Winkel ist die Hülle über alle erreichten, keiner, wenn keines erreicht wird.",
    ),
};

const WALKING: &[&str] = &["walking-surface", "type-hierarchy"];
const NO_WALKING_SURFACE: &str = "the object has no walking surface the service can measure";
const UNDECIDED_KIND: &str = "an object's kind cannot be decided";

const fn kinds(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::SourceKind,
        required: true,
        default: None,
        help,
    }
}

const FLIGHT: &[&str] = &["walking-surface", "type-hierarchy"];
const NO_FLIGHT: &str = "the walking-surface service cannot measure the flight or ramp";
const LANDING_UNMEASURED: &str = "the landing fills no rectangle along the walking direction";

const LANDING_OBJECTS: MeasuredParameter = kinds(
    "landing",
    &en_de(
        "The source kinds that may carry a landing, `,`-separated, subtypes included.",
        "Die Quellarten, die ein Podest tragen können, durch `,` getrennt, Untertypen \
         eingeschlossen.",
    ),
);

const LANDING_END: MeasuredParameter = MeasuredParameter {
    key: "end",
    kind: MeasuredParameterKind::Choice {
        options: &["bottom", "top"],
    },
    required: true,
    default: None,
    help: &en_de(
        "Which end: a flight's bottom or top, a ramp's lowest run's bottom or highest \
         run's top.",
        "Welches Ende: Fuß oder Kopf eines Laufs, Fuß des untersten oder Kopf des \
         obersten Rampenlaufs.",
    ),
};

const WALKING_KIND: MeasuredParameter = MeasuredParameter {
    key: "of",
    kind: MeasuredParameterKind::Choice {
        options: &["flight", "ramp"],
    },
    required: false,
    default: Some("flight"),
    help: &en_de(
        "Whether the object is a stair flight or a ramp.",
        "Ob das Objekt ein Treppenlauf oder eine Rampe ist.",
    ),
};

const WELL_MEMBERS: MeasuredParameter = MeasuredParameter {
    key: "members",
    kind: MeasuredParameterKind::Path,
    required: true,
    default: None,
    help: &en_de(
        "The relationship steps from the well to its stacked spaces.",
        "Die Beziehungsschritte vom Schacht zu seinen gestapelten Räumen.",
    ),
};

const fn need_metres(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: true,
        default: None,
        help,
    }
}

const fn optional_kinds(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::SourceKind,
        required: false,
        default: None,
        help,
    }
}

const fn yes_no(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Choice {
            options: &["no", "yes"],
        },
        required: false,
        default: Some("no"),
        help,
    }
}

const DEFECT_RAILS: MeasuredParameter = kinds(
    "rails",
    &en_de(
        "The source kinds that may be handrails, `,`-separated, subtypes included.",
        "Die Quellarten, die Handläufe sein können, durch `,` getrennt, Untertypen \
         eingeschlossen.",
    ),
);
const DEFECT_REACH_ACROSS: MeasuredParameter = need_metres(
    "reach_across",
    &en_de(
        "How far outside the walking surface's sides a rail may run.",
        "Wie weit außerhalb der Seiten der Lauffläche ein Handlauf liegen darf.",
    ),
);
const DEFECT_REACH_ABOVE: MeasuredParameter = need_metres(
    "reach_above",
    &en_de(
        "How far above the pitch line a rail may run.",
        "Wie weit über der Steigungslinie ein Handlauf liegen darf.",
    ),
);
const STAIR_PATH: MeasuredParameter = MeasuredParameter {
    key: "stair",
    kind: MeasuredParameterKind::Path,
    required: false,
    default: None,
    help: &en_de(
        "The relationship steps from a whole stair to its parts; the object is then a stair.",
        "Die Beziehungsschritte von einer ganzen Treppe zu ihren Teilen; das Objekt ist dann \
         eine Treppe.",
    ),
};
const WITHIN_STAIR: MeasuredParameter = MeasuredParameter {
    key: "within",
    kind: MeasuredParameterKind::Path,
    required: false,
    default: None,
    help: &en_de(
        "The relationship steps from a flight to the whole stairs it is part of: the check \
         runs on them, with `stair` and `flights`, and counts what it reports about the flight.",
        "Die Beziehungsschritte von einem Lauf zu den ganzen Treppen, zu denen er gehört: die \
         Prüfung läuft auf ihnen, mit `stair` und `flights`, und zählt, was sie über den Lauf \
         meldet.",
    ),
};
const STAIR_FLIGHTS: MeasuredParameter = optional_kinds(
    "flights",
    &en_de(
        "The source kinds of the stair's flights among its parts.",
        "Die Quellarten der Läufe unter den Teilen der Treppe.",
    ),
);
const DEFECT_LANDING: MeasuredParameter = kinds(
    "landing",
    &en_de(
        "The source kinds that may carry a landing, `,`-separated, subtypes included.",
        "Die Quellarten, die ein Podest tragen können, durch `,` getrennt, Untertypen \
         eingeschlossen.",
    ),
);
const DEFECTS: &[&str] = &[
    "walking-surface",
    "free-space",
    "proximity",
    "type-hierarchy",
];
const DEFECT_OPEN: &str = "the capability's search leaves a defect open";

const PROXIMITY: &[&str] = &["proximity", "type-hierarchy"];

const COUNTERPARTS: MeasuredParameter = MeasuredParameter {
    key: "to",
    kind: MeasuredParameterKind::SourceKind,
    required: true,
    default: None,
    help: &en_de(
        "The source kinds of the counterparts, `,`-separated, subtypes included.",
        "Die Quellarten der Gegenstücke, durch `,` getrennt, Untertypen eingeschlossen.",
    ),
};

const PROJECTION: MeasuredParameter = MeasuredParameter {
    key: "projection",
    kind: MeasuredParameterKind::Choice {
        options: &["minimum_3d", "horizontal", "vertical", "plan_overlap"],
    },
    required: false,
    default: Some("minimum_3d"),
    help: &en_de(
        "Surface to surface in space, in plan, vertically, or as overlap in plan.",
        "Oberfläche zu Oberfläche im Raum, im Grundriss, vertikal oder als \
         Überlappung im Grundriss.",
    ),
};

const VERTICAL_DIRECTION: MeasuredParameter = MeasuredParameter {
    key: "direction",
    kind: MeasuredParameterKind::Choice {
        options: &["either", "above", "below"],
    },
    required: false,
    default: None,
    help: &en_de(
        "With `vertical`: counterparts above, below, or either.",
        "Mit `vertical`: Gegenstücke darüber, darunter oder beides.",
    ),
};

const NO_DISTANCE: &str = "a counterpart's distance could not be read or straddles";

const SPACE: &[&str] = &["space"];
const SPACE_UNMEASURED: &str = "the space service cannot measure this aspect of the space";

const ELEMENTS: MeasuredParameter = MeasuredParameter {
    key: "elements",
    kind: MeasuredParameterKind::SourceKind,
    required: false,
    default: None,
    help: &en_de(
        "The source kinds of the elements counted, `,`-separated; without it, the \
         service's own default.",
        "Die Quellarten der berücksichtigten Bauteile, durch `,` getrennt; ohne sie die \
         Vorgabe des Dienstes.",
    ),
};

const fn objects(
    key: &'static str,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::SourceKind,
        required,
        default: None,
        help,
    }
}

/// Objects measured against, by source kind or through a selector
/// parameter or the anchor of the rule reading the value.
const fn selected(
    key: &'static str,
    required: bool,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Objects,
        required,
        default: None,
        help,
    }
}

const fn metres(
    key: &'static str,
    default: &'static str,
    help: &'static [LocalizedText],
) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: Some(default),
        help,
    }
}

const PLANE: MeasuredParameter = metres(
    "plane",
    "0",
    &en_de(
        "How far from the body's face planes a boundary still counts, in metres.",
        "Wie weit von den Flächenebenen des Körpers eine Begrenzung noch zählt, in Metern.",
    ),
);

const CONTACT: [MeasuredParameter; 5] = [
    objects(
        "with",
        true,
        &en_de(
            "The source kinds of the objects it may touch, `,`-separated.",
            "Die Quellarten der Objekte, die es berühren können, durch `,` getrennt.",
        ),
    ),
    MeasuredParameter {
        key: "side",
        kind: MeasuredParameterKind::Choice {
            options: &["below", "above"],
        },
        required: false,
        default: Some("below"),
        help: &en_de(
            "The face measured: the one `below` or `above`.",
            "Die gemessene Fläche: die untere (`below`) oder obere (`above`).",
        ),
    },
    metres(
        "gap",
        "0",
        &en_de(
            "The largest gap still touching, in metres.",
            "Der größte Spalt, der noch berührt, in Metern.",
        ),
    ),
    metres(
        "intersection",
        "0",
        &en_de(
            "The deepest intersection still touching, in metres.",
            "Die tiefste Durchdringung, die noch berührt, in Metern.",
        ),
    ),
    MeasuredParameter {
        key: "polygon",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: Some("0"),
        help: &en_de(
            "The least contact polygon counted, in square metres.",
            "Das kleinste gezählte Kontaktpolygon, in Quadratmetern.",
        ),
    },
];

const EFFECT: [MeasuredParameter; 4] = [
    objects(
        "sources",
        true,
        &en_de(
            "The source kinds whose effect areas cover, `,`-separated.",
            "Die Quellarten, deren Wirkbereiche bedecken, durch `,` getrennt.",
        ),
    ),
    objects(
        "blockers",
        false,
        &en_de(
            "The source kinds that block an effect, `,`-separated.",
            "Die Quellarten, die eine Wirkung abschirmen, durch `,` getrennt.",
        ),
    ),
    MeasuredParameter {
        key: "reach",
        kind: MeasuredParameterKind::Choice {
            options: &["grown", "travel", "visible"],
        },
        required: false,
        default: Some("grown"),
        help: &en_de(
            "The source's footprint `grown` by the range, or the free region within \
             the range by `travel` or in view (`visible`).",
            "Der Grundriss der Quelle um die Reichweite vergrößert (`grown`) oder der \
             freie Bereich in Reichweite auf dem Weg (`travel`) oder in Sicht \
             (`visible`).",
        ),
    },
    metres(
        "range",
        "0",
        &en_de(
            "The effect's range, in metres.",
            "Die Reichweite der Wirkung, in Metern.",
        ),
    ),
];

const fn stated(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Property,
        required: false,
        default: None,
        help,
    }
}

const DOOR: &[&str] = &["property-resolution", "object-frame", "vertical-extent"];
const NO_DOOR: &str = "what the door states does not give the value";

const FLOOR_PATH: MeasuredParameter = MeasuredParameter {
    key: "floor_path",
    kind: MeasuredParameterKind::Path,
    required: true,
    default: None,
    help: &en_de(
        "The relationship steps from the object to the spaces whose floors count.",
        "Die Beziehungsschritte vom Objekt zu den Räumen, deren Böden zählen.",
    ),
};

const OVER_FLOORS: MeasuredParameter = MeasuredParameter {
    key: "measure",
    kind: MeasuredParameterKind::Choice {
        options: &["greatest", "least"],
    },
    required: false,
    default: Some("greatest"),
    help: &en_de(
        "Over the floors reached: the `greatest` or the `least`.",
        "Über die erreichten Böden: der größte (`greatest`) oder kleinste (`least`).",
    ),
};

const PROFILE_UNREAD: &str = "the body is no single swept profile, or a derived one";

const LEVEL_PATH: MeasuredParameter = MeasuredParameter {
    key: "path",
    kind: MeasuredParameterKind::Path,
    required: false,
    default: None,
    help: &en_de(
        "The relationship steps from the object to its level; without it, the object \
         is the level.",
        "Die Beziehungsschritte vom Objekt zu seinem Geschoss; ohne sie ist das Objekt \
         das Geschoss.",
    ),
};

const DATUM: MeasuredParameter = MeasuredParameter {
    key: "datum",
    kind: MeasuredParameterKind::Length { minimum: -1.0e6 },
    required: false,
    default: Some("0"),
    help: &en_de(
        "The elevation the ground level lies at or above, in metres.",
        "Die Höhe, auf oder über der das Erdgeschoss liegt, in Metern.",
    ),
};

const LEVELS: &[&str] = &["object-frame", "relationship-selection"];
const LEVEL_UNPLACED: &str = "a level's placement is not stated exactly";

const OWN_AXIS: MeasuredParameterKind = MeasuredParameterKind::Choice {
    options: &["own_x", "own_y", "own_z", "x", "y", "z"],
};

const PLAN_AXIS: MeasuredParameterKind = MeasuredParameterKind::Choice {
    options: &["x", "y", "own_x", "own_y"],
};
pub(super) const NO_GEOMETRY: &str = "the object has no body the service can measure";

/// The members paired as `wall-spacing` pairs them.
pub(super) const SPACED_MEMBERS: MeasuredParameter = MeasuredParameter {
    key: "members",
    kind: MeasuredParameterKind::SourceKind,
    required: true,
    default: None,
    help: &en_de(
        "The source kinds of the members paired, such as walls or beams, `,`-separated.",
        "Die Quellarten der gepaarten Bauteile, etwa Wände oder Träger, durch `,` getrennt.",
    ),
};

pub(super) const MEMBER_PATH: MeasuredParameter = MeasuredParameter {
    key: "member_path",
    kind: MeasuredParameterKind::Path,
    required: true,
    default: None,
    help: &en_de(
        "The relationship steps from the object to its members.",
        "Die Beziehungsschritte vom Objekt zu seinen Bauteilen.",
    ),
};

/// An angle in degrees, written as a plain number.
pub(super) const ANGLE_TOLERANCE: MeasuredParameter = MeasuredParameter {
    key: "angle_tolerance",
    kind: MeasuredParameterKind::Length { minimum: 0.0 },
    required: true,
    default: None,
    help: &en_de(
        "How far from parallel two long axes may be and still pair, in degrees, \
         below 45.",
        "Wie weit zwei Längsachsen von parallel abweichen dürfen, um ein Paar zu \
         bilden, in Grad, unter 45.",
    ),
};

pub(super) const PAIRED: &[&str] = &[
    "plan-span",
    "proximity",
    "vertical-extent",
    "relationship-selection",
    "type-hierarchy",
];

const PAIRED_PLAN_AREA: &[&str] = &[
    "plan-span",
    "proximity",
    "vertical-extent",
    "plan-area",
    "relationship-selection",
    "type-hierarchy",
];

/// The fallback an opening's light area is derived by, as `area-ratio`'s
/// `light-area` numerator names it: each key the rule parameter of that
/// name, so a value reads `overall_width=@overall_width`.
const LIGHT: [MeasuredParameter; 8] = [
    stated(
        "stated",
        &en_de(
            "The light area the object states: the first step, where stated.",
            "Die Lichtfläche, die das Objekt angibt: der erste Schritt, wo angegeben.",
        ),
    ),
    MeasuredParameter {
        key: "overall_width",
        kind: MeasuredParameterKind::Property,
        required: true,
        default: None,
        help: &en_de(
            "The length the object states as its overall width.",
            "Die Länge, die das Objekt als Gesamtbreite angibt.",
        ),
    },
    MeasuredParameter {
        key: "overall_height",
        kind: MeasuredParameterKind::Property,
        required: true,
        default: None,
        help: &en_de(
            "The length the object states as its overall height.",
            "Die Länge, die das Objekt als Gesamthöhe angibt.",
        ),
    },
    MeasuredParameter {
        key: "light_area_table",
        kind: MeasuredParameterKind::Table,
        required: false,
        default: None,
        help: &en_de(
            "Rows of `width`, `height`, `light_area` and an optional `type` pattern: the \
             most specific row of the object's overall size and type gives its light area.",
            "Zeilen aus `width`, `height`, `light_area` und einem optionalen `type`-Muster: \
             die spezifischste Zeile zu Gesamtgröße und Typ des Objekts gibt seine \
             Lichtfläche.",
        ),
    },
    stated(
        "light_type",
        &en_de(
            "The type name a row's `type` pattern matches.",
            "Der Typname, den das `type`-Muster einer Zeile prüft.",
        ),
    ),
    MeasuredParameter {
        key: "light_type_path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The relationship steps to the objects stating the type name; without them, \
             the object itself.",
            "Die Beziehungsschritte zu den Objekten, die den Typnamen angeben; ohne sie \
             das Objekt selbst.",
        ),
    },
    MeasuredParameter {
        key: "light_size_tolerance",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "How far a row's size may differ from the overall size, in metres; none \
             when not given.",
            "Wie weit die Größe einer Zeile von der Gesamtgröße abweichen darf, in \
             Metern; keine Abweichung, wenn nicht angegeben.",
        ),
    },
    MeasuredParameter {
        key: "frame_width",
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: false,
        default: None,
        help: &en_de(
            "The frame allowance, in metres: the last step, the overall size less \
             2·(W+H) times it.",
            "Der Rahmenabzug, in Metern: der letzte Schritt, die Gesamtgröße abzüglich \
             2·(B+H) mal diesem Wert.",
        ),
    },
];

/// What `light_area` reads of the fallback.
const LIGHT_READ: MeasuredParameter = MeasuredParameter {
    key: "read",
    kind: MeasuredParameterKind::Choice {
        options: &["area", "stated", "overall"],
    },
    required: false,
    default: Some("area"),
    help: &en_de(
        "The light `area` by the first step giving one; the area only where it is \
         `stated` (none otherwise); or the `overall` area, width times height.",
        "Die Lichtfläche (`area`) nach dem ersten Schritt, der eine ergibt; die Fläche \
         nur, wo sie angegeben ist (`stated`, sonst keine); oder die Gesamtfläche \
         (`overall`), Breite mal Höhe.",
    ),
};

/// Which step `light_step` tells.
const LIGHT_STEP: MeasuredParameter = MeasuredParameter {
    key: "step",
    kind: MeasuredParameterKind::Choice {
        options: &["stated", "table", "frame"],
    },
    required: true,
    default: None,
    help: &en_de(
        "The step: the `stated` area, the `table`, or the `frame` allowance.",
        "Der Schritt: die angegebene Fläche (`stated`), die Tabelle (`table`) oder der \
         Rahmenabzug (`frame`).",
    ),
};

/// Which side `light_size` reads.
const LIGHT_SIDE: MeasuredParameter = MeasuredParameter {
    key: "side",
    kind: MeasuredParameterKind::Choice {
        options: &["width", "height"],
    },
    required: true,
    default: None,
    help: &en_de(
        "The overall `width` or `height`.",
        "Die Gesamtbreite (`width`) oder -höhe (`height`).",
    ),
};

const LIGHT_UNKNOWN: &str = "no step of the fallback gives a light area";
const LIGHT_SIZE_UNKNOWN: &str = "the overall width or height is not stated as a positive length";

/// A rule's traversal, each parameter named as the rule names it
/// (`relationship=@relationship`): what a measured search walks to the
/// objects it measures against.
const TRAVERSAL: [MeasuredParameter; 5] = [
    MeasuredParameter {
        key: "relationship",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The relationship walked from the object.",
            "Die vom Objekt aus begangene Beziehung.",
        ),
    },
    MeasuredParameter {
        key: "direction",
        kind: MeasuredParameterKind::Text,
        required: false,
        default: None,
        help: &en_de(
            "The relationship's direction: `forward` (the default), `backward` or `either`.",
            "Die Richtung der Beziehung: `forward` (Vorgabe), `backward` oder `either`.",
        ),
    },
    MeasuredParameter {
        key: "path",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "Relationship steps walked instead of one relationship.",
            "Beziehungsschritte statt einer Beziehung.",
        ),
    },
    MeasuredParameter {
        key: "follow_chain",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "Whether chains of the relationship are followed.",
            "Ob Ketten der Beziehung verfolgt werden.",
        ),
    },
    MeasuredParameter {
        key: "skip_absent_relationship_ends",
        kind: MeasuredParameterKind::Truth,
        required: false,
        default: None,
        help: &en_de(
            "Whether a relationship end the source does not state is skipped.",
            "Ob ein von der Quelle nicht angegebenes Beziehungsende übersprungen wird.",
        ),
    },
];

/// What `level-spacing` orders a level among, shared by the level values.
const LEVEL_KINDS: MeasuredParameter = MeasuredParameter {
    key: "levels",
    kind: MeasuredParameterKind::SourceKind,
    required: true,
    default: None,
    help: &en_de(
        "The source kinds of the levels, `,`-separated, such as storeys.",
        "Die Quellarten der Geschosse, durch `,` getrennt, etwa Geschosse eines Gebäudes.",
    ),
};

const LEVEL_ORDER: MeasuredParameter = MeasuredParameter {
    key: "order",
    kind: MeasuredParameterKind::Property,
    required: true,
    default: None,
    help: &en_de(
        "The length every level states its elevation in, such as `Levels/Elevation`.",
        "Die Länge, in der jedes Geschoss seine Höhenlage angibt, etwa `Levels/Elevation`.",
    ),
};

const LEVEL_ANCHOR: MeasuredParameter = MeasuredParameter {
    key: "anchor",
    kind: MeasuredParameterKind::Path,
    required: false,
    default: None,
    help: &en_de(
        "The relationship steps from an anchor (a building) to its levels, walked back \
         from the level; without it, every level of the source.",
        "Die Beziehungsschritte von einem Anker (einem Gebäude) zu seinen Geschossen, vom \
         Geschoss zurück gegangen; ohne sie alle Geschosse der Quelle.",
    ),
};

const LEVEL_ENDS: [MeasuredParameter; 4] = [
    MeasuredParameter {
        key: "lowest",
        kind: MeasuredParameterKind::Choice {
            options: &["checked", "ignored"],
        },
        required: false,
        default: Some("checked"),
        help: &en_de(
            "Whether the lowest level is `checked` or `ignored` (a basement): an ignored \
             level has no rise.",
            "Ob das unterste Geschoss geprüft (`checked`) oder übergangen (`ignored`, ein \
             Keller) wird: ein übergangenes Geschoss hat keine Steighöhe.",
        ),
    },
    MeasuredParameter {
        key: "highest",
        kind: MeasuredParameterKind::Choice {
            options: &["undecided", "ignored"],
        },
        required: false,
        default: Some("undecided"),
        help: &en_de(
            "The highest level, with no level above it: `undecided` unless measured from \
             its contents, or `ignored`, without a rise.",
            "Das höchste Geschoss, ohne Geschoss darüber: unentschieden (`undecided`), \
             sofern nicht aus seinem Inhalt gemessen, oder übergangen (`ignored`), ohne \
             Steighöhe.",
        ),
    },
    MeasuredParameter {
        key: "contents",
        kind: MeasuredParameterKind::Path,
        required: false,
        default: None,
        help: &en_de(
            "The relationship steps from the highest level to its contents, whose highest \
             top less its elevation is its rise.",
            "Die Beziehungsschritte vom höchsten Geschoss zu seinem Inhalt, dessen höchste \
             Oberkante weniger seiner Höhenlage seine Steighöhe ist.",
        ),
    },
    MeasuredParameter {
        key: "content_kinds",
        kind: MeasuredParameterKind::SourceKind,
        required: false,
        default: None,
        help: &en_de(
            "The source kinds of the contents counted, `,`-separated; without it, every \
             object reached.",
            "Die Quellarten des berücksichtigten Inhalts, durch `,` getrennt; ohne sie jedes \
             erreichte Objekt.",
        ),
    },
];

const LEVEL_UNORDERED: &str = "a level of the anchor states no length to order it by, or the \
     level reaches several anchors";
const LEVEL_SERVICES: &[&str] = &[
    "property-resolution",
    "relationship-selection",
    "vertical-extent",
];

const fn shelf_length(key: &'static str, help: &'static [LocalizedText]) -> MeasuredParameter {
    MeasuredParameter {
        key,
        kind: MeasuredParameterKind::Length { minimum: 0.0 },
        required: true,
        default: None,
        help,
    }
}

/// The shelving arrangement and the doors it keeps clear, as
/// `shelf-capacity` declares them.
const SHELVING: [MeasuredParameter; 10] = [
    shelf_length(
        "depth",
        &en_de(
            "The shelves' depth, in metres.",
            "Die Tiefe der Regale, in Metern.",
        ),
    ),
    shelf_length(
        "horizontal",
        &en_de(
            "The horizontal spacing between shelf runs, in metres.",
            "Der waagerechte Abstand zwischen Regalreihen, in Metern.",
        ),
    ),
    shelf_length(
        "vertical",
        &en_de(
            "The vertical spacing between shelf boards, in metres.",
            "Der senkrechte Abstand zwischen Regalböden, in Metern.",
        ),
    ),
    shelf_length(
        "bottom",
        &en_de(
            "The lowest board's elevation above the floor, in metres.",
            "Die Höhe des untersten Bodens über dem Fußboden, in Metern.",
        ),
    ),
    shelf_length(
        "top",
        &en_de(
            "The shelving's top elevation above the floor, in metres.",
            "Die Oberkante der Regale über dem Fußboden, in Metern.",
        ),
    ),
    shelf_length(
        "clearance",
        &en_de(
            "The clearance kept in front of each door or opening, in metres.",
            "Der vor jeder Tür oder Öffnung frei gehaltene Abstand, in Metern.",
        ),
    ),
    MeasuredParameter {
        key: "access",
        kind: MeasuredParameterKind::Path,
        required: true,
        default: None,
        help: &en_de(
            "The relationship steps from a door or opening to the spaces it reaches.",
            "Die Beziehungsschritte von einer Tür oder Öffnung zu den Räumen, die sie \
             erreicht.",
        ),
    },
    selected(
        "doors",
        false,
        &en_de(
            "The doors: their source kinds, `,`-separated, or `@` a selector parameter \
             of the rule.",
            "Die Türen: ihre Quellarten, durch `,` getrennt, oder mit `@` ein \
             Selektorparameter der Regel.",
        ),
    ),
    selected(
        "openings",
        false,
        &en_de(
            "The openings: their source kinds, `,`-separated, or `@` a selector \
             parameter of the rule.",
            "Die Öffnungen: ihre Quellarten, durch `,` getrennt, oder mit `@` ein \
             Selektorparameter der Regel.",
        ),
    ),
    selected(
        "spaces",
        false,
        &en_de(
            "The spaces a door or opening may reach: their source kinds, `,`-separated, \
             or `@` a selector parameter of the rule; without it, every object.",
            "Die Räume, die eine Tür oder Öffnung erreichen kann: ihre Quellarten, durch \
             `,` getrennt, oder mit `@` ein Selektorparameter der Regel; ohne sie jedes \
             Objekt.",
        ),
    ),
];

const SHELVING_UNMEASURED: &[&str] = &[
    "the linear-quantity service cannot measure the space",
    "an element that may reach the space has unreadable spaces or an undecided kind",
    "a selector parameter it names picks objects it cannot list",
];

/// The storeys `slab-contact` orders by their `Elevation` attribute, and the
/// path to the object's one storey.
const STOREYS: [MeasuredParameter; 2] = [
    MeasuredParameter {
        key: "levels",
        kind: MeasuredParameterKind::SourceKind,
        required: true,
        default: None,
        help: &en_de(
            "The source kinds of the storeys, `,`-separated.",
            "Die Quellarten der Geschosse, durch `,` getrennt.",
        ),
    },
    MeasuredParameter {
        key: "path",
        kind: MeasuredParameterKind::Path,
        required: true,
        default: None,
        help: &en_de(
            "The relationship steps from the object to its one storey.",
            "Die Beziehungsschritte vom Objekt zu seinem einen Geschoss.",
        ),
    },
];

const STOREYS_UNORDERED: &str = "a storey states no length `Elevation`, or the object reaches no \
     storey or several";

/// The source a coordinate system is compared with.
const REFERENCE_SOURCE: MeasuredParameter = MeasuredParameter {
    key: "reference",
    kind: MeasuredParameterKind::Text,
    required: false,
    default: None,
    help: &en_de(
        "The discipline of the one reference source; without it, the first source in \
         identity order.",
        "Die Disziplin der einen Referenzquelle; ohne sie die erste Quelle in der \
         Reihenfolge der Kennungen.",
    ),
};

const COORDINATES: &[&str] = &["coordinate-system"];
const UNCOMPARED: &str = "only one of the two sources makes the statement, or a coordinate \
     system cannot be read";
const UNGEOREFERENCED: &str = "a source states no map conversion (not recorded)";

macro_rules! plain {
    ($name:expr, $dimension:expr, $services:expr, $exactness:expr, $not:expr,
     $label:expr, $help:expr) => {
        MeasuredDescriptor {
            name: $name,
            parameters: &[],
            dimension: $dimension,
            services: $services,
            exactness: $exactness,
            not_evaluated: $not,
            label: &$label,
            help: &$help,
        }
    };
}

/// The host axes a face is measured in, as `opening-area` reads them.
pub(super) const FACE_AXES: [MeasuredParameter; 2] = [
    MeasuredParameter {
        key: "length_axis",
        kind: MeasuredParameterKind::Choice {
            options: &["extrusion", "profile-x", "profile-y"],
        },
        required: false,
        default: Some("extrusion"),
        help: &en_de(
            "The host's axis along its length.",
            "Die Achse des Wirts entlang seiner Länge.",
        ),
    },
    MeasuredParameter {
        key: "height_axis",
        kind: MeasuredParameterKind::Choice {
            options: &["extrusion", "profile-x", "profile-y"],
        },
        required: false,
        default: Some("profile-y"),
        help: &en_de(
            "The host's axis along its height.",
            "Die Achse des Wirts entlang seiner Höhe.",
        ),
    },
];

const OPENINGS_PATH: MeasuredParameter = MeasuredParameter {
    key: "path",
    kind: MeasuredParameterKind::Path,
    required: true,
    default: None,
    help: &en_de(
        "The relationship steps from the host to its openings.",
        "Die Beziehungsschritte vom Wirt zu seinen Öffnungen.",
    ),
};

pub(super) const OPENINGS_MINIMUM: MeasuredParameter = MeasuredParameter {
    key: "minimum",
    kind: MeasuredParameterKind::Length { minimum: 0.0 },
    required: false,
    default: None,
    help: &en_de(
        "Openings smaller than this many square metres are not counted.",
        "Öffnungen kleiner als so viele Quadratmeter werden nicht gezählt.",
    ),
};

/// A host's openings and the face they void, as `opening-area` and
/// `empty-host` read them.
const HOST_OPENINGS: &[MeasuredParameter] =
    &[OPENINGS_PATH, FACE_AXES[0], FACE_AXES[1], OPENINGS_MINIMUM];

const ALIGNMENT: MeasuredParameter = MeasuredParameter {
    key: "alignment",
    kind: MeasuredParameterKind::SourceKind,
    required: true,
    default: None,
    help: &en_de(
        "The source kinds of the alignment, `,`-separated; subtypes match. The one \
         object of these kinds the path reaches, or without a path the one in the \
         object's source.",
        "Die Quellarten der Achse, durch `,` getrennt; Untertypen zählen mit. Das eine \
         Objekt dieser Arten, das der Pfad erreicht, oder ohne Pfad das eine in der \
         Quelle des Objekts.",
    ),
};

const ALIGNMENT_PATH: MeasuredParameter = MeasuredParameter {
    key: "path",
    kind: MeasuredParameterKind::Path,
    required: false,
    default: None,
    help: &en_de(
        "The relationship steps from the object to its alignment; without it, every \
         object of the alignment's kinds in the object's source is a candidate.",
        "Die Beziehungsschritte vom Objekt zu seiner Achse; ohne sie ist jedes Objekt \
         der Arten der Achse in der Quelle des Objekts ein Kandidat.",
    ),
};

const ALIGNMENT_SERVICES: &[&str] = &["alignment", "type-hierarchy", "relationship-selection"];
const OFF_RANGE: &str = "the reference point's nearest foot lies before the alignment's start \
     or beyond its end";
const AMBIGUOUS_FOOT: &str = "two feet on the alignment are equally near within the \
     measurement's bounds, or none can be decided";
const ALIGNMENT_UNREAD: &str = "the alignment cannot be read as a centreline, the object's \
     reference point cannot be resolved, or not exactly one alignment is selected";
const PARAMETER_UNBOUNDED: &str = "a segment of the alignment follows a law the service cannot \
     bound, such as a transition spiral of an unsupported family";

/// The three positions along an alignment take the same parameters.
const ALONG: &[MeasuredParameter] = &[ALIGNMENT, ALIGNMENT_PATH];

const STATION: MeasuredParameter = MeasuredParameter {
    key: "station",
    kind: MeasuredParameterKind::Length { minimum: -1.0e6 },
    required: true,
    default: None,
    help: &en_de(
        "The station of the section, in metres, as the alignment labels it through its \
         station equations.",
        "Die Station des Schnitts, in Metern, wie die Achse sie über ihre \
         Stationsgleichungen bezeichnet.",
    ),
};

const SECTION_SERVICES: &[&str] = &["alignment", "type-hierarchy", "relationship-selection"];
const SECTION_UNREAD: &str = "the alignment cannot be read as a centreline, not exactly one \
     alignment is selected, or no station or several distances along it carry the station";
const SECTION_BODY: &str = "the object's body is unmeasured, is no closed solid, or is measured \
     through parts whose overlaps a cut cannot tell apart";
const STATION_OFF_RANGE: &str = "the station lies before the alignment's start or beyond its end";

const ENVELOPE_PARAMETERS: &[MeasuredParameter] = &[
    MeasuredParameter {
        key: "bodies",
        kind: MeasuredParameterKind::SourceKind,
        required: true,
        default: None,
        help: &en_de(
            "The source kinds of the bodies checked against the envelope, `,`-separated; \
             subtypes match. Every object of these kinds in the alignment's source.",
            "Die Quellarten der gegen die Umgrenzung geprüften Körper, durch `,` getrennt; \
             Untertypen zählen mit. Jedes Objekt dieser Arten in der Quelle der Achse.",
        ),
    },
    MeasuredParameter {
        key: "envelope",
        kind: MeasuredParameterKind::Polygon,
        required: true,
        default: None,
        help: &en_de(
            "The clearance envelope in the section, `lateral:up` vertices in metres, \
             `,`-separated: lateral horizontal, positive to the left of the direction of \
             travel, up vertical above the gradient line.",
            "Die Lichtraumumgrenzung im Schnitt, Eckpunkte `seitlich:oben` in Metern, durch \
             `,` getrennt: seitlich waagerecht, positiv links der Stationierungsrichtung, \
             oben lotrecht über der Gradiente.",
        ),
    },
    MeasuredParameter {
        key: "from",
        kind: MeasuredParameterKind::Length { minimum: -1.0e6 },
        required: true,
        default: None,
        help: &en_de(
            "The station the checked range starts at, in metres.",
            "Die Station, an der der geprüfte Bereich beginnt, in Metern.",
        ),
    },
    MeasuredParameter {
        key: "to",
        kind: MeasuredParameterKind::Length { minimum: -1.0e6 },
        required: true,
        default: None,
        help: &en_de(
            "The station the checked range ends at, in metres; not before `from`.",
            "Die Station, an der der geprüfte Bereich endet, in Metern; nicht vor `from`.",
        ),
    },
    MeasuredParameter {
        key: "step",
        kind: MeasuredParameterKind::Length { minimum: 0.001 },
        required: true,
        default: None,
        help: &en_de(
            "The plan distance between sampled sections, in metres. Between two samples the \
             check is bounded, never interpolated; a step too coarse to decide leaves the \
             value undecided.",
            "Der Abstand der geprüften Schnitte im Lageplan, in Metern. Zwischen zwei \
             Schnitten wird die Prüfung abgeschätzt, nie interpoliert; ein zu grober Schritt \
             lässt den Wert unentschieden.",
        ),
    },
];

/// Every measured value, sorted by name.
pub static MEASURED_VALUES: &[MeasuredDescriptor] = &[
    MeasuredDescriptor {
        name: "alignment_cant",
        parameters: ALONG,
        dimension: Some(QuantityDimension::Length),
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            ALIGNMENT_UNREAD,
            OFF_RANGE,
            AMBIGUOUS_FOOT,
            PARAMETER_UNBOUNDED,
        ],
        label: &en_de("Cant", "Überhöhung"),
        help: &en_de(
            "How far one rail head stands above the other at the object's station, \
             unsigned; none when the alignment states no cant.",
            "Wie weit ein Schienenkopf über dem anderen steht, an der Station des Objekts, \
             ohne Vorzeichen; keine, wenn die Achse keine Überhöhung angibt.",
        ),
    },
    MeasuredDescriptor {
        name: "alignment_curvature",
        parameters: ALONG,
        dimension: None,
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            ALIGNMENT_UNREAD,
            OFF_RANGE,
            AMBIGUOUS_FOOT,
            PARAMETER_UNBOUNDED,
        ],
        label: &en_de("Horizontal curvature", "Krümmung im Lageplan"),
        help: &en_de(
            "The plan curvature of the alignment at the object's station, per metre: positive \
             turning left, zero on a straight.",
            "Die Krümmung der Achse im Lageplan an der Station des Objekts, je Meter: positiv \
             nach links, null auf einer Geraden.",
        ),
    },
    MeasuredDescriptor {
        name: "alignment_gradient",
        parameters: ALONG,
        dimension: None,
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            ALIGNMENT_UNREAD,
            OFF_RANGE,
            AMBIGUOUS_FOOT,
            PARAMETER_UNBOUNDED,
        ],
        label: &en_de("Gradient", "Längsneigung"),
        help: &en_de(
            "The gradient line's rise over plan run at the object's station, positive rising \
             in the direction of travel.",
            "Steigung der Gradiente über der Lagelänge an der Station des Objekts, positiv \
             steigend in Stationierungsrichtung.",
        ),
    },
    MeasuredDescriptor {
        name: "alignment_radius",
        parameters: ALONG,
        dimension: Some(QuantityDimension::Length),
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            ALIGNMENT_UNREAD,
            OFF_RANGE,
            AMBIGUOUS_FOOT,
            PARAMETER_UNBOUNDED,
            "the alignment may run straight there, so its radius is unbounded",
        ],
        label: &en_de("Horizontal radius", "Radius im Lageplan"),
        help: &en_de(
            "The plan radius of the alignment at the object's station, unsigned; not evaluated \
             where the alignment may run straight there.",
            "Der Radius der Achse im Lageplan an der Station des Objekts, ohne Vorzeichen; nicht \
             ausgewertet, wo die Achse dort gerade verlaufen kann.",
        ),
    },
    MeasuredDescriptor {
        name: ANGLE_TO,
        parameters: &[
            MeasuredParameter {
                key: "between",
                kind: MeasuredParameterKind::Choice {
                    options: &["axis", "face_normal"],
                },
                required: false,
                default: Some("axis"),
                help: &en_de(
                    "`axis`: the acute angle between the footprints' long axes in plan; \
                     `face_normal`: the angle between the top faces' normals.",
                    "`axis`: der spitze Winkel zwischen den Längsachsen der Grundrisse; \
                     `face_normal`: der Winkel zwischen den Normalen der Oberseiten.",
                ),
            },
            REFERENCE_PATH,
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: &["plan-span", "relationship-selection", "vertical-extent"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_LONG_AXIS, STANDS_VERTICAL],
        label: &en_de("Angle to", "Winkel zu"),
        help: &en_de(
            "The angle between the object and the objects a path reaches.",
            "Der Winkel zwischen dem Objekt und den Objekten, die ein Pfad erreicht.",
        ),
    },
    plain!(
        MEASURED_AREA,
        Some(QuantityDimension::Area),
        &["plan-area"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Footprint area", "Grundfläche"),
        en_de(
            "The area of the object's footprint in plan, overlaps counted once.",
            "Die Fläche des Grundrisses des Objekts, Überlappungen einmal gezählt."
        )
    ),
    MeasuredDescriptor {
        name: "band_uncovered_area",
        parameters: &[
            SPACED_MEMBERS,
            MEMBER_PATH,
            ANGLE_TOLERANCE,
            MeasuredParameter {
                key: "maximum",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The farthest apart a parallel pair may stand and still bound a \
                     band, in metres.",
                    "Der größte Abstand, mit dem ein paralleles Paar noch ein Band \
                     begrenzt, in Metern.",
                ),
            },
            objects(
                "footprints",
                true,
                &en_de(
                    "The source kinds of the footprint objects, such as slabs, \
                     `,`-separated.",
                    "Die Quellarten der Grundrissobjekte, etwa Decken, durch `,` getrennt.",
                ),
            ),
            MeasuredParameter {
                key: "footprint_path",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from the object to its footprint objects.",
                    "Die Beziehungsschritte vom Objekt zu seinen Grundrissobjekten.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Area),
        services: PAIRED_PLAN_AREA,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the path reaches no footprint object",
            "a member's extent cannot be read",
        ],
        label: &en_de("Area outside bands", "Fläche außerhalb der Bänder"),
        help: &en_de(
            "The largest area of a footprint the object reaches that lies outside every \
             band between parallel members at most `maximum` apart, as `wall-spacing` \
             measures its coverage.",
            "Die größte Fläche eines erreichten Grundrissobjekts außerhalb aller Bänder \
             zwischen parallelen Bauteilen mit höchstens `maximum` Abstand, wie \
             `wall-spacing` die Abdeckung misst.",
        ),
    },
    MeasuredDescriptor {
        name: BEARING,
        parameters: &[
            MeasuredParameter {
                key: "axis",
                kind: MeasuredParameterKind::Choice {
                    options: &["own_x", "own_y", "long"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "`own_x` or `own_y` of the placement in plan, or the footprint's \
                     `long` axis, which has no direction (a bearing in `[0, π)`).",
                    "`own_x` oder `own_y` der Platzierung im Grundriss oder die `long` \
                     Längsachse des Grundrisses, die keine Richtung hat (in `[0, π)`).",
                ),
            },
            MeasuredParameter {
                key: "reference",
                kind: MeasuredParameterKind::Choice {
                    options: &["project_north", "true_north"],
                },
                required: false,
                default: Some("project_north"),
                help: &en_de(
                    "`project_north`, the plan y axis, or `true_north` as the source \
                     states it.",
                    "`project_north`, die y-Achse des Plans, oder `true_north`, wie die \
                     Quelle sie angibt.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: &["object-frame", "plan-span", "coordinate-system"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "the placement is not stated exactly",
            NO_LONG_AXIS,
            "the source states no true north",
            "the axis stands vertical",
        ],
        label: &en_de("Bearing", "Ausrichtung"),
        help: &en_de(
            "The plan bearing of an axis, clockwise from north.",
            "Die Grundrissrichtung einer Achse, im Uhrzeigersinn von Norden.",
        ),
    },
    MeasuredDescriptor {
        name: "body_extent",
        parameters: &[MeasuredParameter {
            key: "axis",
            kind: MeasuredParameterKind::Choice {
                options: &["right", "forward", "up"],
            },
            required: true,
            default: None,
            help: &en_de(
                "The placement's own axis: `right` (its first), `forward` (its second, \
                 across a wall whose layers run along its first) or `up`.",
                "Die eigene Achse der Platzierung: `right` (ihre erste), `forward` (ihre \
                 zweite, quer zu einer Wand, deren Schichten entlang der ersten laufen) \
                 oder `up`.",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: FACES_AND_FRAMES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the object's placement is not stated or not readable",
        ],
        label: &en_de("Body extent", "Körperausdehnung"),
        help: &en_de(
            "The body's depth along one of its own placement axes, as `body-extent` \
             measures it: the highest less the lowest point projected onto the axis.",
            "Die Tiefe des Körpers entlang einer eigenen Achse seiner Platzierung, wie \
             `body-extent` sie misst: der höchste weniger der tiefste auf die Achse \
             projizierte Punkt.",
        ),
    },
    MeasuredDescriptor {
        name: "body_position",
        parameters: &[
            MeasuredParameter {
                key: "axis",
                kind: MeasuredParameterKind::Choice {
                    options: &["right", "forward", "up"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The placement's own axis, as for `body_extent`.",
                    "Die eigene Achse der Platzierung, wie bei `body_extent`.",
                ),
            },
            MeasuredParameter {
                key: "end",
                kind: MeasuredParameterKind::Choice {
                    options: &["low", "high"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "`low`, the body's lowest point projected onto the axis, or `high`, its \
                     highest.",
                    "`low`, der tiefste auf die Achse projizierte Punkt des Körpers, oder \
                     `high`, der höchste.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: FACES_AND_FRAMES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the object's placement is not stated or not readable",
        ],
        label: &en_de("Body position", "Körperlage"),
        help: &en_de(
            "Where the body begins or ends along one of its own placement axes: its lowest \
             or highest point projected onto the axis, measured from the origin of the \
             coordinates along it. `body_extent` is the high less the low end; the \
             positions are the magnitudes the binary rounding of that difference scales \
             with.",
            "Wo der Körper entlang einer eigenen Achse seiner Platzierung beginnt oder endet: \
             sein tiefster oder höchster auf die Achse projizierter Punkt, gemessen vom \
             Ursprung der Koordinaten entlang der Achse. `body_extent` ist das hohe weniger \
             das tiefe Ende; die Lagen sind die Größen, mit denen die binäre Rundung dieser \
             Differenz wächst.",
        ),
    },
    plain!(
        MEASURED_BOTTOM,
        Some(QuantityDimension::Length),
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Bottom elevation", "Unterkante"),
        en_de(
            "The elevation of the object's lowest point.",
            "Die Höhe des tiefsten Punkts des Objekts."
        )
    ),
    MeasuredDescriptor {
        name: MEASURED_BOTTOM_ABOVE_LEVEL,
        parameters: &[MeasuredParameter {
            key: "path",
            kind: MeasuredParameterKind::Path,
            required: true,
            default: None,
            help: &en_de(
                "The relationship steps from the object to its level.",
                "Die Beziehungsschritte vom Objekt zu seinem Geschoss.",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: &["relationship-selection", "object-frame", "vertical-extent"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "a reached level's placement is not stated exactly",
            "the path reaches levels at different elevations",
        ],
        label: &en_de("Bottom above level", "Unterkante über Geschoss"),
        help: &en_de(
            "The object's bottom above the placement origin of the one level the path \
             reaches; none when it reaches no level.",
            "Die Unterkante des Objekts über dem Ursprung des einen Geschosses, das der \
             Pfad erreicht; keine, wenn er kein Geschoss erreicht.",
        ),
    },
    MeasuredDescriptor {
        name: MEASURED_BOUNDARY_AREA,
        parameters: &[
            MeasuredParameter {
                key: "kind",
                kind: MeasuredParameterKind::SourceKind,
                required: true,
                default: None,
                help: &en_de(
                    "The kind of bounding element, subtypes included.",
                    "Die Art des begrenzenden Bauteils, Untertypen eingeschlossen.",
                ),
            },
            MeasuredParameter {
                key: "plane",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "How far from the body's face planes a boundary still counts, in metres.",
                    "Wie weit von den Flächenebenen des Körpers eine Begrenzung noch zählt, \
                     in Metern.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["boundary-coverage", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "a boundary names no bounding element",
            "a bounding element's kind cannot be decided",
            NO_GEOMETRY,
        ],
        label: &en_de("Boundary area", "Begrenzungsfläche"),
        help: &en_de(
            "A space's summed space-boundary area against elements of one kind.",
            "Die summierte Raumbegrenzungsfläche eines Raums gegen Bauteile einer Art.",
        ),
    },
    MeasuredDescriptor {
        name: "boundary_covered_share",
        parameters: &[PLANE],
        dimension: None,
        services: &["boundary-coverage"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Covered boundary share", "Bedeckter Randanteil"),
        help: &en_de(
            "The share of a space body's surface its declared space boundaries cover, from 0 to 1.",
            "Der Anteil der Oberfläche eines Raumkörpers, den seine Raumbegrenzungen bedecken, von 0 bis 1.",
        ),
    },
    MeasuredDescriptor {
        name: "boundary_gap",
        parameters: &[
            ELEMENTS,
            MeasuredParameter {
                key: "at_least",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "Only gaps at least this long count, in metres.",
                    "Nur Lücken mindestens dieser Länge zählen, in Metern.",
                ),
            },
            MeasuredParameter {
                key: "measure",
                kind: MeasuredParameterKind::Choice {
                    options: &["total", "longest"],
                },
                required: false,
                default: Some("total"),
                help: &en_de(
                    "Their `total` length, or the `longest`.",
                    "Ihre Gesamtlänge (`total`) oder die längste (`longest`).",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: SPACE,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SPACE_UNMEASURED],
        label: &en_de("Uncovered boundary", "Unbedeckter Raumrand"),
        help: &en_de(
            "How much of a space's boundary no element covers, as `space-validation` \
             counts it.",
            "Wie viel vom Rand eines Raums kein Bauteil bedeckt, wie \
             `space-validation` es zählt.",
        ),
    },
    MeasuredDescriptor {
        name: "boundary_off_surface_count",
        parameters: &[PLANE],
        dimension: None,
        services: &["boundary-coverage"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de(
            "Boundaries off the surface",
            "Begrenzungen neben der Oberfläche",
        ),
        help: &en_de(
            "How many of a space's declared boundaries lie on no face of its body, so \
             cover nothing.",
            "Wie viele Raumbegrenzungen eines Raums auf keiner Fläche seines Körpers \
             liegen und daher nichts bedecken.",
        ),
    },
    MeasuredDescriptor {
        name: "boundary_overlap_area",
        parameters: &[PLANE],
        dimension: Some(QuantityDimension::Area),
        services: &["boundary-coverage"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de(
            "Overlapping boundary area",
            "Überlappende Begrenzungsfläche",
        ),
        help: &en_de(
            "The area a space's declared boundaries cover twice.",
            "Die Fläche, die die Raumbegrenzungen doppelt bedecken.",
        ),
    },
    MeasuredDescriptor {
        name: "boundary_uncovered_area",
        parameters: &[PLANE],
        dimension: Some(QuantityDimension::Area),
        services: &["boundary-coverage"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Uncovered boundary area", "Unbedeckte Begrenzungsfläche"),
        help: &en_de(
            "The area of a space body's surface no declared boundary covers.",
            "Die Fläche der Oberfläche eines Raumkörpers, die keine Raumbegrenzung bedeckt.",
        ),
    },
    MeasuredDescriptor {
        name: "cap_coverage",
        parameters: &[
            MeasuredParameter {
                key: "cap",
                kind: MeasuredParameterKind::Choice {
                    options: &["top", "bottom"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The `top` or `bottom` cap.",
                    "Die obere oder untere Abdeckung.",
                ),
            },
            ELEMENTS,
        ],
        dimension: None,
        services: SPACE,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SPACE_UNMEASURED],
        label: &en_de("Cap coverage", "Deckenabdeckung"),
        help: &en_de(
            "The share of a space's top or bottom the elements cover, from 0 to 1; none \
             when no element of the kinds named exists.",
            "Der Anteil der Ober- oder Unterseite eines Raums, den die Bauteile bedecken, \
             von 0 bis 1; keiner, wenn kein Bauteil der genannten Arten besteht.",
        ),
    },
    MeasuredDescriptor {
        name: "centre_line_distance",
        parameters: &[
            kinds(
                "walls",
                &en_de(
                    "The source kinds of the walls beside it, `,`-separated, subtypes included.",
                    "Die Quellarten der Wände daneben, durch `,` getrennt, Untertypen \
                     eingeschlossen.",
                ),
            ),
            MeasuredParameter {
                key: "centre_line",
                kind: MeasuredParameterKind::Choice {
                    options: &["long", "short", "against-wall"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The footprint's centre line: along its long or short axis, or from the \
                     wall it stands against to its front.",
                    "Die Mittellinie des Grundrisses: entlang seiner langen oder kurzen Achse \
                     oder von der Wand, vor der es steht, zu seiner Vorderseite.",
                ),
            },
            MeasuredParameter {
                key: "side",
                kind: MeasuredParameterKind::Choice {
                    options: &["nearest", "farther"],
                },
                required: false,
                default: Some("nearest"),
                help: &en_de(
                    "The nearer of the two sides' walls, or the farther: every side's wall is \
                     within the farther's distance.",
                    "Die nähere der Wände beider Seiten oder die fernere: jede Seite hat ihre \
                     Wand innerhalb des Abstands der ferneren.",
                ),
            },
            MeasuredParameter {
                key: "reach",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "How far from the centre line a wall is searched.",
                    "Wie weit von der Mittellinie eine Wand gesucht wird.",
                ),
            },
            metres(
                "inset",
                "0",
                &en_de(
                    "How far the strip beside the footprint is narrowed at both ends.",
                    "Um wie viel der Streifen neben dem Grundriss an beiden Enden verkürzt \
                     wird.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["plan-span", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "the footprint has no long axis",
            "no side is surely nearer a wall than the others",
        ],
        label: &en_de(
            "Distance from the centre line",
            "Abstand von der Mittellinie",
        ),
        help: &en_de(
            "How far the centre line of the footprint's least-area rectangle lies from the \
             nearest wall beside it, square to it; just past `reach` where a wall may but \
             need not lie, none where none may.",
            "Wie weit die Mittellinie des kleinsten umschließenden Rechtecks des Grundrisses \
             von der nächsten Wand daneben liegt, rechtwinklig zu ihr; knapp jenseits von \
             `reach`, wo eine Wand liegen kann, aber nicht muss, keiner, wo keine liegen kann.",
        ),
    },
    plain!(
        CLEAR_HEIGHT,
        Some(QuantityDimension::Length),
        &["space"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY, "the space's floor or ceiling cannot be found"],
        en_de("Clear height", "Lichte Höhe"),
        en_de(
            "A space's clear height, as the space service measures it for \
             `space-validation`.",
            "Die lichte Höhe eines Raums, wie der Raumdienst sie für \
             `space-validation` misst."
        )
    ),
    MeasuredDescriptor {
        name: CLEAR_WIDTH,
        parameters: &[
            kinds(
                "obstacles",
                &en_de(
                    "The source kinds that narrow it, `,`-separated, subtypes included: \
                     walls, handrails, anything beside or over it.",
                    "Die Quellarten, die sie einengen, durch `,` getrennt, Untertypen \
                     eingeschlossen: Wände, Handläufe, alles daneben oder darüber.",
                ),
            ),
            MeasuredParameter {
                key: "band_from",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "The band's bottom above the pitch line, in metres.",
                    "Die Unterkante des Bands über der Steigungslinie, in Metern.",
                ),
            },
            MeasuredParameter {
                key: "band_to",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The band's top above the pitch line, in metres.",
                    "Die Oberkante des Bands über der Steigungslinie, in Metern.",
                ),
            },
            MeasuredParameter {
                key: "along",
                kind: MeasuredParameterKind::Choice {
                    options: &["flight", "runs"],
                },
                required: false,
                default: Some("flight"),
                help: &en_de(
                    "A stair `flight`, or a ramp's `runs` (the least over them).",
                    "Ein Treppenlauf (`flight`) oder die Läufe einer Rampe (`runs`, der \
                     kleinste).",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: WALKING,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_WALKING_SURFACE, UNDECIDED_KIND],
        label: &en_de("Clear width", "Lichte Breite"),
        help: &en_de(
            "The narrowest free width across a flight or a ramp's runs between two \
             heights above its pitch line, as `stair-geometry` measures it.",
            "Die kleinste freie Breite quer über einen Lauf oder die Läufe einer Rampe \
             zwischen zwei Höhen über der Steigungslinie, wie `stair-geometry` sie misst.",
        ),
    },
    MeasuredDescriptor {
        name: CLEARANCE_BELOW,
        parameters: &[kinds(
            "spaces",
            &en_de(
                "The source kinds of the spaces whose floors are below, `,`-separated.",
                "Die Quellarten der Räume, deren Böden darunter liegen, durch `,` getrennt.",
            ),
        )],
        dimension: Some(QuantityDimension::Length),
        services: WALKING,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_WALKING_SURFACE,
            UNDECIDED_KIND,
            "the object crosses a floor",
        ],
        label: &en_de("Clearance below", "Durchgangshöhe darunter"),
        help: &en_de(
            "The least height between a flight's or ramp's underside and the floors \
             of the spaces below; none when it stands above none.",
            "Die kleinste Höhe zwischen der Unterseite eines Laufs oder einer Rampe und \
             den Böden der Räume darunter; keine, wenn sie über keinem steht.",
        ),
    },
    MeasuredDescriptor {
        name: "contact_area",
        parameters: &CONTACT,
        dimension: Some(QuantityDimension::Area),
        services: &["contact", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Contact area", "Kontaktfläche"),
        help: &en_de(
            "The area of a face in contact with the objects of the kinds named, as `slab-contact` measures it.",
            "Die Fläche einer Seite in Kontakt mit Objekten der genannten Arten, wie `slab-contact` sie misst.",
        ),
    },
    MeasuredDescriptor {
        name: "contact_gap",
        parameters: &CONTACT,
        dimension: Some(QuantityDimension::Length),
        services: &["contact", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the body has no direction the service can orient",
        ],
        label: &en_de("Contact gap", "Kontaktabstand"),
        help: &en_de(
            "How far the nearest object of the kinds named lies from the face, as the \
             contact service reports it with the contact area; none where it reports \
             none near.",
            "Wie weit das nächste Objekt der genannten Arten von der Fläche entfernt ist, \
             wie der Kontaktdienst es mit der Kontaktfläche meldet; keiner, wo er keines \
             in der Nähe meldet.",
        ),
    },
    MeasuredDescriptor {
        name: "contact_share",
        parameters: &CONTACT,
        dimension: None,
        services: &["contact", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Contact share", "Kontaktanteil"),
        help: &en_de(
            "The share of a face in contact, from 0 to 1, as `slab-contact` judges it.",
            "Der Anteil einer Seite in Kontakt, von 0 bis 1, wie `slab-contact` ihn beurteilt.",
        ),
    },
    MeasuredDescriptor {
        name: "coordinate_shift",
        parameters: &[
            MeasuredParameter {
                key: "of",
                kind: MeasuredParameterKind::Choice {
                    options: &["world", "site", "map"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The world frame's origin, the site placement's origin, or the map \
                     conversion's offset in metres.",
                    "Der Ursprung des Weltrahmens, der Ursprung der Grundstücksplatzierung \
                     oder der Versatz der Kartenumrechnung in Metern.",
                ),
            },
            REFERENCE_SOURCE,
        ],
        dimension: Some(QuantityDimension::Length),
        services: COORDINATES,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            UNCOMPARED,
            "the map unit is not stated exactly",
            UNGEOREFERENCED,
        ],
        label: &en_de("Coordinate shift", "Koordinatenversatz"),
        help: &en_de(
            "How far the object's source moves a statement of its coordinate system from \
             the reference source's, as `coordinate-consistency` compares them; none where \
             neither states a site.",
            "Wie weit die Quelle des Objekts eine Angabe ihres Koordinatensystems gegenüber \
             der Referenzquelle verschiebt, wie `coordinate-consistency` sie vergleicht; \
             keiner, wo keine ein Grundstück angibt.",
        ),
    },
    MeasuredDescriptor {
        name: "coordinate_turn",
        parameters: &[
            MeasuredParameter {
                key: "of",
                kind: MeasuredParameterKind::Choice {
                    options: &["world", "site", "north", "map"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The world frame's axes, the site placement's axes, true north, or the \
                     map conversion's rotation.",
                    "Die Achsen des Weltrahmens, die Achsen der Grundstücksplatzierung, \
                     geografisch Nord oder die Drehung der Kartenumrechnung.",
                ),
            },
            REFERENCE_SOURCE,
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: COORDINATES,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[UNCOMPARED, UNGEOREFERENCED],
        label: &en_de("Coordinate turn", "Koordinatendrehung"),
        help: &en_de(
            "How far the object's source turns a statement of its coordinate system from \
             the reference source's, as `coordinate-consistency` compares them; none where \
             neither states a site or true north.",
            "Wie weit die Quelle des Objekts eine Angabe ihres Koordinatensystems gegenüber \
             der Referenzquelle dreht, wie `coordinate-consistency` sie vergleicht; keine, \
             wo keine ein Grundstück oder geografisch Nord angibt.",
        ),
    },
    MeasuredDescriptor {
        name: COUNT_WITHIN,
        parameters: &[
            COUNTERPARTS,
            MeasuredParameter {
                key: "radius",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The greatest distance counted, in metres.",
                    "Der größte gezählte Abstand, in Metern.",
                ),
            },
            MeasuredParameter {
                key: "from",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "The least distance counted, in metres.",
                    "Der kleinste gezählte Abstand, in Metern.",
                ),
            },
            PROJECTION,
            VERTICAL_DIRECTION,
        ],
        dimension: None,
        services: PROXIMITY,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, UNDECIDED_KIND],
        label: &en_de("Count within", "Anzahl im Umkreis"),
        help: &en_de(
            "How many counterparts lie within a radius: an interval from those surely \
             within to those possibly within, a point when every one is decided.",
            "Wie viele Gegenstücke im Umkreis liegen: ein Intervall von den sicher bis zu \
             den möglicherweise darin liegenden, ein Punkt, wenn alle entschieden sind.",
        ),
    },
    MeasuredDescriptor {
        name: "counterpart_uncovered_share",
        parameters: &[
            objects(
                "by",
                true,
                &en_de(
                    "The source kinds of the counterparts, such as structural walls and \
                     columns, `,`-separated, subtypes included.",
                    "Die Quellarten der Gegenstücke, etwa tragende Wände und Stützen, durch \
                     `,` getrennt, Untertypen eingeschlossen.",
                ),
            ),
            MeasuredParameter {
                key: "measure",
                kind: MeasuredParameterKind::Choice {
                    options: &["plan", "height", "elevation"],
                },
                required: false,
                default: Some("plan"),
                help: &en_de(
                    "The share of the footprint (`plan`), of the height (`height`), or of \
                     the elevation along the footprint's long axis (`elevation`).",
                    "Der Anteil der Grundfläche (`plan`), der Höhe (`height`) oder der \
                     Ansicht entlang der Längsachse des Grundrisses (`elevation`).",
                ),
            },
            MeasuredParameter {
                key: "horizontal",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "How far each counterpart grows in plan, or along the axis in the \
                     elevation, in metres; for `height`, how far it grows to overlap in plan.",
                    "Wie weit jedes Gegenstück im Grundriss wächst, in der Ansicht entlang \
                     der Achse, in Metern; für `height`, wie weit es wächst, um im Grundriss \
                     zu überlappen.",
                ),
            },
            MeasuredParameter {
                key: "vertical",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "How far each counterpart grows in height, in metres; unused in plan.",
                    "Wie weit jedes Gegenstück in der Höhe wächst, in Metern; im Grundriss \
                     ungenutzt.",
                ),
            },
            MeasuredParameter {
                key: "axis_tolerance",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "Only counterparts whose long axis lies within this many degrees of \
                     parallel count, below 45; any angle without it.",
                    "Nur Gegenstücke, deren Längsachse höchstens so viele Grad von parallel \
                     abweicht, zählen, unter 45; ohne Angabe jeder Winkel.",
                ),
            },
            objects(
                "frame",
                false,
                &en_de(
                    "The source kinds of a frame's members whose infill covers too, in the \
                     elevation only, `,`-separated.",
                    "Die Quellarten der Rahmenbauteile, deren Ausfachung ebenfalls bedeckt, \
                     nur in der Ansicht, durch `,` getrennt.",
                ),
            ),
            MeasuredParameter {
                key: "infill_above",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0.5"),
                help: &en_de(
                    "The share, below 1, above which the frame's infill covers.",
                    "Der Anteil, unter 1, ab dem die Ausfachung des Rahmens bedeckt.",
                ),
            },
        ],
        dimension: None,
        services: &[
            "plan-area",
            "proximity",
            "vertical-extent",
            "plan-span",
            "type-hierarchy",
        ],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            UNDECIDED_KIND,
            "the element has no long axis for its elevation",
        ],
        label: &en_de(
            "Share left uncovered by counterparts",
            "Von Gegenstücken unbedeckter Anteil",
        ),
        help: &en_de(
            "The share of the element's footprint, height or elevation outside every \
             counterpart, from 0 to 1, as `counterpart-coverage` measures it: from the most \
             cover counterparts may give to the least they surely give.",
            "Der Anteil der Grundfläche, Höhe oder Ansicht des Bauteils außerhalb aller \
             Gegenstücke, von 0 bis 1, wie `counterpart-coverage` ihn misst: von der größten \
             möglichen bis zur sicheren Bedeckung.",
        ),
    },
    MeasuredDescriptor {
        name: CROSS_FALL,
        parameters: &[
            MeasuredParameter {
                key: "axis",
                kind: PLAN_AXIS,
                required: true,
                default: None,
                help: &en_de(
                    "The plan axis the fall is read across: world `x` or `y`, or the \
                     object's own `own_x` or `own_y` in plan.",
                    "Die Grundrissachse, quer zu der das Gefälle gelesen wird: `x` oder \
                     `y` der Welt oder die eigene `own_x` oder `own_y` im Grundriss.",
                ),
            },
            FACE,
            FACING[0],
            FACING[1],
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES_AND_FRAMES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_FACE, STANDS_VERTICAL, FACING_UNDECIDED],
        label: &en_de("Cross fall", "Quergefälle"),
        help: &en_de(
            "The face's gradient across a plan axis, unsigned, as an angle; the hull \
             over its pieces.",
            "Das Gefälle der Fläche quer zu einer Grundrissachse, ohne Vorzeichen, als \
             Winkel; die Hülle über ihre Teile.",
        ),
    },
    MeasuredDescriptor {
        name: DISTANCE,
        parameters: &[
            COUNTERPARTS,
            MeasuredParameter {
                key: "mode",
                kind: MeasuredParameterKind::Choice {
                    options: &["nearest", "farthest"],
                },
                required: false,
                default: Some("nearest"),
                help: &en_de(
                    "The nearest or the farthest counterpart.",
                    "Das nächste oder das fernste Gegenstück.",
                ),
            },
            PROJECTION,
            VERTICAL_DIRECTION,
            MeasuredParameter {
                key: "subject_surface",
                kind: MeasuredParameterKind::Choice {
                    options: &["top", "bottom"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "With `vertical`: from the object's top or bottom; with \
                     `counterpart_surface`.",
                    "Mit `vertical`: von Ober- oder Unterseite des Objekts; mit \
                     `counterpart_surface`.",
                ),
            },
            MeasuredParameter {
                key: "counterpart_surface",
                kind: MeasuredParameterKind::Choice {
                    options: &["top", "bottom", "nearest"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "With `vertical`: to the counterpart's top, bottom, or the surface \
                     directly over or under the object.",
                    "Mit `vertical`: zur Ober- oder Unterseite des Gegenstücks oder der \
                     Fläche direkt darüber oder darunter.",
                ),
            },
            MeasuredParameter {
                key: "within",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("1000"),
                help: &en_de(
                    "How far counterparts are searched for, in metres; none within is no \
                     distance.",
                    "Wie weit nach Gegenstücken gesucht wird, in Metern; keines darin ist \
                     kein Abstand.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: PROXIMITY,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, UNDECIDED_KIND, NO_DISTANCE],
        label: &en_de("Distance", "Abstand"),
        help: &en_de(
            "The nearest or farthest counterpart's distance, as `distance` measures \
             it; an undecided counterpart widens it towards the bound it could move.",
            "Der Abstand zum nächsten oder fernsten Gegenstück, wie `distance` ihn misst; \
             ein unentschiedenes Gegenstück weitet ihn zur Grenze, die es verschieben \
             könnte.",
        ),
    },
    MeasuredDescriptor {
        name: "door_clear_height",
        parameters: &[
            stated(
                "stated",
                &en_de(
                    "The clear height the door states.",
                    "Die angegebene lichte Höhe der Tür.",
                ),
            ),
            stated(
                "overall",
                &en_de(
                    "The overall height the door states.",
                    "Die angegebene Gesamthöhe der Tür.",
                ),
            ),
            stated(
                "lining",
                &en_de(
                    "The head lining thickness it states.",
                    "Die angegebene Sturzbekleidung.",
                ),
            ),
            stated(
                "threshold",
                &en_de(
                    "The threshold thickness it states.",
                    "Die angegebene Schwellenhöhe.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: DOOR,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[NO_DOOR],
        label: &en_de("Door clear height", "Lichte Türhöhe"),
        help: &en_de(
            "A door's clear height, as `keyed-limit`'s `clear-height` reads it: stated, \
             else the overall height less its lining and threshold.",
            "Die lichte Höhe einer Tür, wie `keyed-limit` sie als `clear-height` liest: \
             angegeben, sonst die Gesamthöhe weniger Sturzbekleidung und Schwelle.",
        ),
    },
    MeasuredDescriptor {
        name: "door_clear_width",
        parameters: &[
            stated(
                "stated",
                &en_de(
                    "The clear width the door states.",
                    "Die angegebene lichte Breite der Tür.",
                ),
            ),
            MeasuredParameter {
                key: "from_leaves",
                kind: MeasuredParameterKind::Choice {
                    options: &["passage", "widest-leaf"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "Derive it from the leaves: the whole `passage`, or the `widest-leaf`.",
                    "Aus den Flügeln ableiten: der ganze Durchgang (`passage`) oder der \
                     breiteste Flügel (`widest-leaf`).",
                ),
            },
            stated(
                "overall",
                &en_de(
                    "The overall width the door states.",
                    "Die angegebene Gesamtbreite der Tür.",
                ),
            ),
            MeasuredParameter {
                key: "deduction",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "The deduction from the overall width, in metres.",
                    "Der Abzug von der Gesamtbreite, in Metern.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: DOOR,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[NO_DOOR],
        label: &en_de("Door clear width", "Lichte Türbreite"),
        help: &en_de(
            "A door's clear width, as `keyed-limit`'s `clear-width` reads it: stated, \
             else from its leaves, else the overall width less the deduction.",
            "Die lichte Breite einer Tür, wie `keyed-limit` sie als `clear-width` liest: \
             angegeben, sonst aus den Flügeln, sonst die Gesamtbreite weniger dem Abzug.",
        ),
    },
    plain!(
        "duplicate_count",
        None,
        SPACE,
        MeasuredExactness::Measured,
        &[SPACE_UNMEASURED],
        en_de("Duplicates", "Duplikate"),
        en_de(
            "How many other spaces' bodies coincide with the space's.",
            "Wie viele andere Räume denselben Körper wie der Raum haben."
        )
    ),
    MeasuredDescriptor {
        name: "effect_covered_area",
        parameters: &EFFECT,
        dimension: Some(QuantityDimension::Area),
        services: &["plan-area", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Effect-covered area", "Wirkbedeckte Fläche"),
        help: &en_de(
            "The part of the footprint the sources' effect areas cover, as `effective-coverage` measures it.",
            "Der Teil des Grundrisses, den die Wirkbereiche der Quellen bedecken, wie `effective-coverage` ihn misst.",
        ),
    },
    MeasuredDescriptor {
        name: "effect_covered_share",
        parameters: &EFFECT,
        dimension: None,
        services: &["plan-area", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Effect-covered share", "Wirkbedeckter Anteil"),
        help: &en_de(
            "The share of the footprint the sources' effect areas cover, from 0 to 1.",
            "Der Anteil des Grundrisses, den die Wirkbereiche der Quellen bedecken, von 0 bis 1.",
        ),
    },
    MeasuredDescriptor {
        name: "end_width",
        parameters: &[LANDING_END],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, "the width at that end is not measured"],
        label: &en_de("Width at an end", "Breite an einem Ende"),
        help: &en_de(
            "The width a landing at one end of a flight is compared with: the flight's, or \
             a turning flight's tread meeting it.",
            "Die Breite, mit der ein Podest an einem Ende eines Laufs verglichen wird: die \
             des Laufs oder bei einem gewendelten Lauf die der angrenzenden Stufe.",
        ),
    },
    MeasuredDescriptor {
        name: "envelope_intrusions",
        parameters: ENVELOPE_PARAMETERS,
        dimension: None,
        services: &["alignment", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "the object is no alignment the service can read as a centreline, or a station \
             of the range is carried by no distance or by several",
            "the range lies partly before the alignment's start or beyond its end",
            "a selected body is unmeasured or is no closed solid",
        ],
        label: &en_de(
            "Clearance envelope intrusions",
            "Eingriffe in die Lichtraumumgrenzung",
        ),
        help: &en_de(
            "How many of the selected bodies reach into the clearance envelope swept along \
             the alignment from `from` to `to`, measured on the alignment: the bodies surely \
             intruding to those that may. Between sampled sections the envelope is grown by \
             how far it can move, so a body crossing it between two samples is never missed; \
             one that cannot be decided widens the count.",
            "Wie viele der ausgewählten Körper in die längs der Achse von `from` bis `to` \
             geführte Lichtraumumgrenzung hineinragen, gemessen an der Achse: von den sicher \
             hineinragenden bis zu denen, die hineinragen können. Zwischen den Schnitten wird \
             die Umgrenzung um ihre mögliche Bewegung vergrößert, sodass ein Körper zwischen \
             zwei Schnitten nie übersehen wird; ein unentscheidbarer verbreitert die Anzahl.",
        ),
    },
    MeasuredDescriptor {
        name: EXTENT,
        parameters: &[
            MeasuredParameter {
                key: "axis",
                kind: OWN_AXIS,
                required: false,
                default: None,
                help: &en_de(
                    "An own axis of the placement (`own_x`, `own_y`, `own_z`) or a world \
                     axis (`x`, `y`, `z`); this or `direction`.",
                    "Eine eigene Achse der Platzierung (`own_x`, `own_y`, `own_z`) oder \
                     eine Weltachse (`x`, `y`, `z`); dies oder `direction`.",
                ),
            },
            MeasuredParameter {
                key: "direction",
                kind: MeasuredParameterKind::Vector,
                required: false,
                default: None,
                help: &en_de(
                    "A world direction `x,y,z`; this or `axis`.",
                    "Eine Weltrichtung `x,y,z`; dies oder `axis`.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "object-frame"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, "the placement is not stated exactly"],
        label: &en_de("Extent", "Ausdehnung"),
        help: &en_de(
            "The body's depth along an axis: its highest less its lowest point \
             projected onto it.",
            "Die Tiefe des Körpers entlang einer Achse: sein höchster weniger sein \
             tiefster Punkt, auf sie projiziert.",
        ),
    },
    plain!(
        MEASURED_EXTENT_X,
        Some(QuantityDimension::Length),
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Extent along x", "Ausdehnung in x"),
        en_de(
            "The body's extent along the world x axis.",
            "Die Ausdehnung des Körpers entlang der x-Achse."
        )
    ),
    plain!(
        MEASURED_EXTENT_Y,
        Some(QuantityDimension::Length),
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Extent along y", "Ausdehnung in y"),
        en_de(
            "The body's extent along the world y axis.",
            "Die Ausdehnung des Körpers entlang der y-Achse."
        )
    ),
    plain!(
        MEASURED_EXTENT_Z,
        Some(QuantityDimension::Length),
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Height", "Höhe"),
        en_de(
            "The body's vertical extent.",
            "Die vertikale Ausdehnung des Körpers."
        )
    ),
    MeasuredDescriptor {
        name: "facade_area",
        parameters: &[],
        dimension: Some(QuantityDimension::Area),
        services: &["facade-area"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Facade area", "Fassadenfläche"),
        help: &en_de(
            "The object's facade area, as `plan-area` with `measure` `facade` reads it.",
            "Die Fassadenfläche des Objekts, wie `plan-area` mit `measure` `facade` sie liest.",
        ),
    },
    MeasuredDescriptor {
        name: "face_area",
        parameters: &[],
        dimension: Some(QuantityDimension::Area),
        services: &["facade-area"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Face area", "Seitenfläche"),
        help: &en_de(
            "The area of the object's largest plane face: a wall's or slab's side.",
            "Die Fläche der größten ebenen Seite des Objekts: die Seite einer Wand oder Decke.",
        ),
    },
    MeasuredDescriptor {
        name: "flight_rise",
        parameters: &[super::members::WALKING_LINE_OFFSET],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT],
        label: &en_de("Flight rise", "Laufhöhe"),
        help: &en_de(
            "How far a stair flight rises, from its base to the top of its last riser.",
            "Wie hoch ein Treppenlauf steigt, von seinem Fuß bis zur Oberkante der letzten \
             Steigung.",
        ),
    },
    MeasuredDescriptor {
        name: "flight_width",
        parameters: &[],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, "a tread fills no rectangle along the flight"],
        label: &en_de("Flight width", "Laufbreite"),
        help: &en_de(
            "A stair flight's width: its narrowest tread's, across the direction it climbs.",
            "Die Breite eines Treppenlaufs: die seiner schmalsten Stufe quer zur \
             Laufrichtung.",
        ),
    },
    MeasuredDescriptor {
        name: GRADIENT_DIRECTION,
        parameters: &[FACE, FACING[0], FACING[1]],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            NO_FACE,
            STANDS_VERTICAL,
            FACING_UNDECIDED,
            "a piece of the face is level, so it descends nowhere",
            "the face descends in directions more than a half turn apart",
        ],
        label: &en_de("Direction of descent", "Fallrichtung"),
        help: &en_de(
            "The plan bearing of the face's steepest descent, clockwise from plan \
             north (the y axis).",
            "Die Grundrissrichtung des steilsten Gefälles der Fläche, im Uhrzeigersinn \
             von Plannord (der y-Achse).",
        ),
    },
    MeasuredDescriptor {
        name: "handrail_breaks",
        parameters: &[
            DEFECT_RAILS,
            DEFECT_REACH_ACROSS,
            DEFECT_REACH_ABOVE,
            WALKING_KIND,
            STAIR_PATH,
            STAIR_FLIGHTS,
            optional_kinds(
                "break_doors",
                &en_de(
                    "The source kinds of doors at which a rail may break.",
                    "Die Quellarten der Türen, an denen ein Handlauf enden darf.",
                ),
            ),
            MeasuredParameter {
                key: "landing",
                kind: MeasuredParameterKind::SourceKind,
                required: false,
                default: None,
                help: &en_de(
                    "The source kinds that may carry a landing, for `break_doors`.",
                    "Die Quellarten, die ein Podest tragen können, für `break_doors`.",
                ),
            },
            MeasuredParameter {
                key: "height",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "How high over a landing a break door stands, for `break_doors`.",
                    "Wie hoch über einem Podest eine Unterbrechungstür steht, für `break_doors`.",
                ),
            },
            MeasuredParameter {
                key: "tolerance",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "The gap a ramp's rail may leave across a landing.",
                    "Die Lücke, die der Handlauf einer Rampe über einem Podest lassen darf.",
                ),
            },
        ],
        dimension: None,
        services: DEFECTS,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, DEFECT_OPEN],
        label: &en_de("Handrail breaks", "Handlaufunterbrechungen"),
        help: &en_de(
            "How many times the handrail along a whole stair breaks across its landings, or along a ramp across its landings between runs, as `stair-geometry` and `ramp-geometry` judge it.",
            "Wie oft der Handlauf entlang einer ganzen Treppe über ihren Podesten oder entlang einer Rampe über den Podesten zwischen ihren Läufen unterbrochen ist, wie `stair-geometry` und `ramp-geometry` es beurteilen.",
        ),
    },
    MeasuredDescriptor {
        name: HEADROOM,
        parameters: &[kinds(
            "obstacles",
            &en_de(
                "The source kinds that may stand above, `,`-separated, subtypes included.",
                "Die Quellarten, die darüber stehen können, durch `,` getrennt, \
                 Untertypen eingeschlossen.",
            ),
        )],
        dimension: Some(QuantityDimension::Length),
        services: WALKING,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_WALKING_SURFACE,
            UNDECIDED_KIND,
            "an obstacle crosses the surface",
        ],
        label: &en_de("Headroom", "Kopfhöhe"),
        help: &en_de(
            "The least vertical clearance above a walking surface; none when nothing \
             selected stands above.",
            "Der kleinste lichte Abstand über einer Lauffläche; keiner, wenn nichts \
             Ausgewähltes darüber steht.",
        ),
    },
    MeasuredDescriptor {
        name: "height_above_gradient",
        parameters: ALONG,
        dimension: Some(QuantityDimension::Length),
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[ALIGNMENT_UNREAD, OFF_RANGE, AMBIGUOUS_FOOT],
        label: &en_de("Height above gradient", "Höhe über Gradiente"),
        help: &en_de(
            "The reference point's elevation above the alignment's gradient line at its foot.",
            "Die Höhe des Bezugspunkts über der Gradiente der Achse an seinem Lotfußpunkt.",
        ),
    },
    MeasuredDescriptor {
        name: "height_above_ground",
        parameters: &[LEVEL_PATH, DATUM],
        dimension: Some(QuantityDimension::Length),
        services: LEVELS,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[LEVEL_UNPLACED, "no level lies at or above the datum"],
        label: &en_de("Height above ground", "Höhe über Erdgeschoss"),
        help: &en_de(
            "The level's elevation above the source's ground level: the lowest of its \
             levels at or above the datum.",
            "Die Höhe des Geschosses über dem Erdgeschoss der Quelle: dem niedrigsten \
             Geschoss auf oder über der Bezugshöhe.",
        ),
    },
    MeasuredDescriptor {
        name: INCLINATION,
        parameters: &[MeasuredParameter {
            key: "axis",
            kind: MeasuredParameterKind::Choice {
                options: &["own_x", "own_y", "own_z"],
            },
            required: true,
            default: None,
            help: &en_de(
                "The object's own axis: `own_z` is measured from the vertical, \
                 `own_x` and `own_y` from the horizontal.",
                "Die eigene Achse des Objekts: `own_z` wird von der Senkrechten \
                 gemessen, `own_x` und `own_y` von der Waagerechten.",
            ),
        }],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: &["object-frame"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &["the placement is not stated exactly"],
        label: &en_de("Inclination", "Neigung"),
        help: &en_de(
            "The tilt of one of the object's placement axes.",
            "Die Neigung einer Achse der Platzierung des Objekts.",
        ),
    },
    MeasuredDescriptor {
        name: "intersection_count",
        parameters: &[
            ELEMENTS,
            MeasuredParameter {
                key: "tolerance",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0.005"),
                help: &en_de(
                    "A partial overlap no higher than this is no intersection, in metres.",
                    "Eine Teilüberlappung, die nicht höher ist, ist keine Durchdringung, \
                     in Metern.",
                ),
            },
        ],
        dimension: None,
        services: SPACE,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SPACE_UNMEASURED],
        label: &en_de("Intersections", "Durchdringungen"),
        help: &en_de(
            "How many bodies contain, are contained by or substantially intersect the \
             space, as `space-validation` counts them.",
            "Wie viele Körper den Raum enthalten, in ihm liegen oder ihn wesentlich \
             durchdringen, wie `space-validation` sie zählt.",
        ),
    },
    MeasuredDescriptor {
        name: "landing_clear_width",
        parameters: &[
            LANDING_OBJECTS,
            LANDING_END,
            kinds(
                "obstacles",
                &en_de(
                    "The source kinds that narrow it, `,`-separated, subtypes included.",
                    "Die Quellarten, die sie einengen, durch `,` getrennt, Untertypen \
                     eingeschlossen.",
                ),
            ),
            need_metres(
                "band_from",
                &en_de(
                    "The band's bottom above the landing's level.",
                    "Die Unterkante des Bands über der Podesthöhe.",
                ),
            ),
            need_metres(
                "band_to",
                &en_de(
                    "The band's top above the landing's level.",
                    "Die Oberkante des Bands über der Podesthöhe.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_FLIGHT,
            "no selected obstacle bounds a side of the landing",
        ],
        label: &en_de("Landing clear width", "Lichte Podestbreite"),
        help: &en_de(
            "The clear width of the landing at one end of a flight between two heights above \
             its level, across the direction leaving the flight; none when no selected object \
             carries one.",
            "Die lichte Breite des Podests an einem Ende eines Laufs zwischen zwei Höhen über \
             seiner Höhe, quer zur Richtung aus dem Lauf; keine, wenn kein ausgewähltes Objekt \
             eines trägt.",
        ),
    },
    MeasuredDescriptor {
        name: "landing_count",
        parameters: &[LANDING_OBJECTS, LANDING_END, WALKING_KIND],
        dimension: None,
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, UNDECIDED_KIND],
        label: &en_de("Landings at an end", "Podeste an einem Ende"),
        help: &en_de(
            "Whether a landing of the kinds meets one end of a flight or ramp: 1 or 0.",
            "Ob ein Podest der Arten ein Ende eines Laufs oder einer Rampe trifft: 1 oder 0.",
        ),
    },
    MeasuredDescriptor {
        name: "landing_depth",
        parameters: &[LANDING_OBJECTS, LANDING_END, WALKING_KIND],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, UNDECIDED_KIND, LANDING_UNMEASURED],
        label: &en_de("Landing depth", "Podesttiefe"),
        help: &en_de(
            "How deep the landing at one end of a flight or ramp is along the walking \
             direction; none when no selected object carries one.",
            "Wie tief das Podest an einem Ende eines Laufs oder einer Rampe in \
             Gehrichtung ist; keine, wenn kein ausgewähltes Objekt eines trägt.",
        ),
    },
    MeasuredDescriptor {
        name: "landing_door_conflicts",
        parameters: &[
            DEFECT_LANDING,
            kinds(
                "doors",
                &en_de(
                    "The source kinds of doors, `,`-separated.",
                    "Die Quellarten der Türen, durch `,` getrennt.",
                ),
            ),
            need_metres(
                "height",
                &en_de(
                    "How high the column over each landing reaches.",
                    "Wie hoch die Säule über jedem Podest reicht.",
                ),
            ),
            yes_no(
                "swing",
                &en_de(
                    "Whether a door swinging over a landing counts too.",
                    "Ob auch eine über ein Podest aufschlagende Tür zählt.",
                ),
            ),
            WALKING_KIND,
        ],
        dimension: None,
        services: DEFECTS,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, DEFECT_OPEN],
        label: &en_de("Doors at landings", "Türen an Podesten"),
        help: &en_de(
            "How many landings at the ends of a flight or ramp a door stands in the column over, or with `swing=yes` swings over.",
            "An wie vielen Podesten an den Enden eines Laufs oder einer Rampe eine Tür in der Säule darüber steht oder mit `swing=yes` darüber aufschlägt.",
        ),
    },
    MeasuredDescriptor {
        name: "landing_width",
        parameters: &[LANDING_OBJECTS, LANDING_END, WALKING_KIND],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, UNDECIDED_KIND, LANDING_UNMEASURED],
        label: &en_de("Landing width", "Podestbreite"),
        help: &en_de(
            "How wide the landing at one end of a flight or ramp is across the walking \
             direction; none when no selected object carries one.",
            "Wie breit das Podest an einem Ende eines Laufs oder einer Rampe quer zur \
             Gehrichtung ist; keine, wenn kein ausgewähltes Objekt eines trägt.",
        ),
    },
    plain!(
        "largest_unallocated_region",
        Some(QuantityDimension::Area),
        SPACE,
        MeasuredExactness::Measured,
        &[SPACE_UNMEASURED],
        en_de("Largest unallocated region", "Größte unzugeordnete Fläche"),
        en_de(
            "The largest connected region of a storey's floor no space covers; zero when \
             none is.",
            "Die größte zusammenhängende Geschossfläche, die kein Raum bedeckt; null, wenn \
             keine."
        )
    ),
    plain!(
        "leaf_count",
        None,
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the door states no leaves"],
        en_de("Leaves", "Flügel"),
        en_de(
            "How many leaves a door or window has.",
            "Wie viele Flügel eine Tür oder ein Fenster hat."
        )
    ),
    MeasuredDescriptor {
        name: "leaf_width",
        parameters: &[MeasuredParameter {
            key: "measure",
            kind: MeasuredParameterKind::Choice {
                options: &["widest", "narrowest", "total"],
            },
            required: false,
            default: Some("widest"),
            help: &en_de(
                "The `widest` leaf, the `narrowest`, or their `total`.",
                "Der breiteste Flügel (`widest`), der schmalste (`narrowest`) oder ihre \
                 Summe (`total`).",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: &["object-frame"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &["the door states no leaves"],
        label: &en_de("Leaf width", "Flügelbreite"),
        help: &en_de(
            "A width of a door's or window's leaves.",
            "Eine Breite der Flügel einer Tür oder eines Fensters.",
        ),
    },
    MeasuredDescriptor {
        name: LENGTH,
        parameters: &[MeasuredParameter {
            key: "axis",
            kind: MeasuredParameterKind::Choice {
                options: &["own_x", "own_y", "own_z"],
            },
            required: false,
            default: Some("own_x"),
            help: &en_de(
                "The member's own axis it is swept along: `own_x` for a beam or member \
                 as placed, `own_z` for a column.",
                "Die eigene Achse, entlang der das Bauteil verläuft: `own_x` für einen \
                 Träger, `own_z` für eine Stütze.",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "object-frame"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, "the placement is not stated exactly"],
        label: &en_de("Length", "Länge"),
        help: &en_de(
            "A member's length: its body's extent along its own sweep axis.",
            "Die Länge eines Bauteils: die Ausdehnung seines Körpers entlang seiner \
             eigenen Achse.",
        ),
    },
    MeasuredDescriptor {
        name: "level_elevation",
        parameters: &[LEVEL_PATH],
        dimension: Some(QuantityDimension::Length),
        services: LEVELS,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            LEVEL_UNPLACED,
            "the path reaches levels at different elevations",
        ],
        label: &en_de("Level elevation", "Geschosshöhe über Null"),
        help: &en_de(
            "The elevation of the level's placement origin; none when the path reaches no \
             level.",
            "Die Höhe des Platzierungsursprungs des Geschosses; keine, wenn der Pfad kein \
             Geschoss erreicht.",
        ),
    },
    plain!(
        MEASURED_LEVEL_HEIGHT,
        Some(QuantityDimension::Length),
        &["property-resolution"],
        MeasuredExactness::Stated,
        &["the source does not state storey elevations"],
        en_de("Storey height", "Geschosshöhe"),
        en_de(
            "A storey's height to the next storey of the same spatial parent, as the \
             source states it; none for the highest storey.",
            "Die Höhe eines Geschosses bis zum nächsten Geschoss desselben räumlichen \
             Elternteils, wie die Quelle sie angibt; keine für das oberste Geschoss."
        )
    ),
    MeasuredDescriptor {
        name: "level_index",
        parameters: &[LEVEL_PATH, DATUM],
        dimension: None,
        services: LEVELS,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[LEVEL_UNPLACED, "no level lies at or above the datum"],
        label: &en_de("Level index", "Geschossindex"),
        help: &en_de(
            "The level's index counted from the ground level: 0 on it, up above and \
             negative below; levels at one elevation share an index.",
            "Der Index des Geschosses vom Erdgeschoss aus: 0 auf ihm, aufwärts darüber, \
             negativ darunter; Geschosse gleicher Höhe teilen einen Index.",
        ),
    },
    MeasuredDescriptor {
        name: "level_rise",
        parameters: &[
            LEVEL_KINDS,
            LEVEL_ORDER,
            LEVEL_ANCHOR,
            LEVEL_PATH,
            LEVEL_ENDS[0],
            LEVEL_ENDS[1],
            LEVEL_ENDS[2],
            LEVEL_ENDS[3],
        ],
        dimension: Some(QuantityDimension::Length),
        services: LEVEL_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            LEVEL_UNORDERED,
            "the highest level's rise is undecided, or its contents cannot be measured",
        ],
        label: &en_de("Level rise", "Geschosshöhe"),
        help: &en_de(
            "A level's height as `level-spacing` measures it: the rise of its `order` to \
             the next level up among its anchor's levels, the highest one's from its \
             contents; none for an ignored level.",
            "Die Höhe eines Geschosses, wie `level-spacing` sie misst: der Anstieg seiner \
             Höhenlage (`order`) zum nächsthöheren Geschoss seines Ankers, die des höchsten \
             aus seinem Inhalt; keine für ein übergangenes Geschoss.",
        ),
    },
    MeasuredDescriptor {
        name: "levels_above",
        parameters: &STOREYS,
        dimension: None,
        services: &["property-resolution", "relationship-selection"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[STOREYS_UNORDERED],
        label: &en_de("Storeys above", "Geschosse darüber"),
        help: &en_de(
            "How many storeys of the object's source lie above its storey by their \
             `Elevation` attribute, a whole number: none on the top storey, as \
             `slab-contact` decides it.",
            "Wie viele Geschosse der Quelle des Objekts nach ihrem Attribut `Elevation` über \
             seinem Geschoss liegen, eine ganze Zahl: keines auf dem obersten Geschoss, wie \
             `slab-contact` es entscheidet.",
        ),
    },
    MeasuredDescriptor {
        name: "levels_below",
        parameters: &STOREYS,
        dimension: None,
        services: &["property-resolution", "relationship-selection"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[STOREYS_UNORDERED],
        label: &en_de("Storeys below", "Geschosse darunter"),
        help: &en_de(
            "How many storeys of the object's source lie below its storey by their \
             `Elevation` attribute, a whole number: none on the bottom storey, as \
             `slab-contact` decides it.",
            "Wie viele Geschosse der Quelle des Objekts nach ihrem Attribut `Elevation` \
             unter seinem Geschoss liegen, eine ganze Zahl: keines auf dem untersten \
             Geschoss, wie `slab-contact` es entscheidet.",
        ),
    },
    MeasuredDescriptor {
        name: "light_area",
        parameters: &[
            LIGHT[0], LIGHT[1], LIGHT[2], LIGHT[3], LIGHT[4], LIGHT[5], LIGHT[6], LIGHT[7],
            LIGHT_READ,
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["property-resolution", "relationship-selection"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[LIGHT_UNKNOWN, LIGHT_SIZE_UNKNOWN],
        label: &en_de("Light area", "Lichtfläche"),
        help: &en_de(
            "An opening's light-transmitting area by the first step of a fallback that \
             gives one, as `area-ratio`'s `light-area` numerator derives it: the area it \
             states, else the most specific light-area row of its overall size and type, \
             else its overall size less the frame allowance.",
            "Die lichtdurchlässige Fläche einer Öffnung nach dem ersten Schritt einer \
             Rückfallkette, der eine ergibt, wie der Zähler `light-area` von `area-ratio` \
             sie ableitet: die angegebene Fläche, sonst die spezifischste Lichtflächenzeile \
             zu Gesamtgröße und Typ, sonst die Gesamtgröße abzüglich des Rahmenabzugs.",
        ),
    },
    MeasuredDescriptor {
        name: "light_size",
        parameters: &[
            LIGHT[0], LIGHT[1], LIGHT[2], LIGHT[3], LIGHT[4], LIGHT[5], LIGHT[6], LIGHT[7],
            LIGHT_SIDE,
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["property-resolution"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[LIGHT_SIZE_UNKNOWN],
        label: &en_de("Overall opening size", "Gesamtgröße der Öffnung"),
        help: &en_de(
            "The overall width or height an opening states, which its light area is \
             compared with and derived from.",
            "Die Gesamtbreite oder -höhe, die eine Öffnung angibt, mit der ihre \
             Lichtfläche verglichen und aus der sie abgeleitet wird.",
        ),
    },
    MeasuredDescriptor {
        name: "light_step",
        parameters: &[
            LIGHT[0], LIGHT[1], LIGHT[2], LIGHT[3], LIGHT[4], LIGHT[5], LIGHT[6], LIGHT[7],
            LIGHT_STEP,
        ],
        dimension: None,
        services: &["property-resolution", "relationship-selection"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[LIGHT_UNKNOWN],
        label: &en_de("Light-area step", "Schritt der Lichtfläche"),
        help: &en_de(
            "1 where the opening's light area comes from the step, 0 where another \
             gives it: summed over openings, how many areas each step gave.",
            "1, wo die Lichtfläche der Öffnung aus dem Schritt stammt, 0, wo ein anderer \
             sie ergibt: über Öffnungen summiert, wie viele Flächen jeder Schritt ergab.",
        ),
    },
    MeasuredDescriptor {
        name: "map_conversion",
        parameters: &[
            MeasuredParameter {
                key: "of",
                kind: MeasuredParameterKind::Choice {
                    options: &["own", "reference"],
                },
                required: false,
                default: Some("own"),
                help: &en_de(
                    "The object's own source, or the reference source.",
                    "Die eigene Quelle des Objekts oder die Referenzquelle.",
                ),
            },
            REFERENCE_SOURCE,
        ],
        dimension: None,
        services: COORDINATES,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &["a coordinate system cannot be read"],
        label: &en_de("Map conversion stated", "Kartenumrechnung angegeben"),
        help: &en_de(
            "1 where the source states a map conversion, 0 where it is not georeferenced.",
            "1, wo die Quelle eine Kartenumrechnung angibt, 0, wo sie nicht georeferenziert \
             ist.",
        ),
    },
    MeasuredDescriptor {
        name: "map_scale_change",
        parameters: &[REFERENCE_SOURCE],
        dimension: None,
        services: COORDINATES,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &["a coordinate system cannot be read", UNGEOREFERENCED],
        label: &en_de("Map scale change", "Änderung des Kartenmaßstabs"),
        help: &en_de(
            "How far the map conversion's scale of the object's source differs from the \
             reference source's, a plain number.",
            "Wie weit der Maßstab der Kartenumrechnung der Quelle des Objekts von dem der \
             Referenzquelle abweicht, eine reine Zahl.",
        ),
    },
    MeasuredDescriptor {
        name: "map_target_change",
        parameters: &[REFERENCE_SOURCE],
        dimension: None,
        services: COORDINATES,
        exactness: MeasuredExactness::Stated,
        not_evaluated: &["a coordinate system cannot be read", UNGEOREFERENCED],
        label: &en_de("Map target change", "Änderung des Zielsystems"),
        help: &en_de(
            "1 where the map conversion of the object's source targets another system than \
             the reference source's, 0 where both name the same.",
            "1, wo die Kartenumrechnung der Quelle des Objekts ein anderes Zielsystem nennt \
             als die der Referenzquelle, 0, wo beide dasselbe nennen.",
        ),
    },
    MeasuredDescriptor {
        name: "middle_face_area",
        parameters: &FACE_AXES,
        dimension: Some(QuantityDimension::Area),
        services: &["body-facts"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            "the host is no straight extrusion the body set bounds",
            "its outline's edge runs along its middle plane",
        ],
        label: &en_de("Middle-plane face area", "Fläche der Mittelebene"),
        help: &en_de(
            "The area of a host's face on its middle plane, as `empty-host` compares its \
             openings with it: the box's length times its height, the outline's area, or \
             the outline's chord along the face times the height.",
            "Die Fläche der Ansicht eines Wirts auf seiner Mittelebene, wie `empty-host` \
             seine Öffnungen mit ihr vergleicht: Länge mal Höhe des Quaders, die Fläche \
             des Umrisses oder seine Sehne entlang der Ansicht mal der Höhe.",
        ),
    },
    MeasuredDescriptor {
        name: "missing_tactile_strips",
        parameters: &[
            kinds(
                "tactiles",
                &en_de(
                    "The source kinds that may be tactile surfaces.",
                    "Die Quellarten, die taktile Flächen sein können.",
                ),
            ),
            need_metres(
                "offset",
                &en_de(
                    "How far before the first riser and beyond the last the strip lies.",
                    "Wie weit vor der ersten und hinter der letzten Steigung der Streifen liegt.",
                ),
            ),
            need_metres(
                "depth",
                &en_de("How deep the strip is.", "Wie tief der Streifen ist."),
            ),
            yes_no(
                "intermediate",
                &en_de(
                    "Whether the landings between a stair's flights need strips too.",
                    "Ob auch die Podeste zwischen den Läufen einer Treppe Streifen brauchen.",
                ),
            ),
            STAIR_PATH,
            STAIR_FLIGHTS,
            WITHIN_STAIR,
        ],
        dimension: None,
        services: DEFECTS,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, DEFECT_OPEN],
        label: &en_de("Missing tactile strips", "Fehlende taktile Streifen"),
        help: &en_de(
            "How many of the tactile strips required before the first riser and beyond the last of a flight (or a whole stair) no selected object covers.",
            "Wie viele der vor der ersten und hinter der letzten Steigung eines Laufs (oder einer ganzen Treppe) geforderten taktilen Streifen kein ausgewähltes Objekt bedeckt.",
        ),
    },
    MeasuredDescriptor {
        name: "obstructed_end_spaces",
        parameters: &[
            kinds(
                "obstacles",
                &en_de(
                    "The source kinds that obstruct an end space.",
                    "Die Quellarten, die einen Freiraum am Ende verstellen.",
                ),
            ),
            need_metres(
                "depth",
                &en_de(
                    "How deep the free space before the end is.",
                    "Wie tief der Freiraum vor dem Ende ist.",
                ),
            ),
            need_metres(
                "width",
                &en_de("How wide the free space is.", "Wie breit der Freiraum ist."),
            ),
            need_metres(
                "height",
                &en_de("How high the free space is.", "Wie hoch der Freiraum ist."),
            ),
            WALKING_KIND,
        ],
        dimension: None,
        services: DEFECTS,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, DEFECT_OPEN],
        label: &en_de("Obstructed end spaces", "Verstellte Freiräume an den Enden"),
        help: &en_de(
            "How many of the free spaces before the first riser and beyond the last of a flight (or before the lowest and beyond the highest run of a ramp) an obstacle reaches into.",
            "In wie viele der Freiräume vor der ersten und hinter der letzten Steigung eines Laufs (oder vor dem untersten und hinter dem obersten Rampenlauf) ein Hindernis reicht.",
        ),
    },
    MeasuredDescriptor {
        name: "obstruction_count",
        parameters: &[
            objects(
                "obstacles",
                true,
                &en_de(
                    "The source kinds that may obstruct, `,`-separated.",
                    "Die Quellarten, die versperren können, durch `,` getrennt.",
                ),
            ),
            MeasuredParameter {
                key: "reach",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "How far from the footprint in plan an obstacle counts, in metres.",
                    "Wie weit vom Grundriss entfernt ein Hindernis zählt, in Metern.",
                ),
            },
            MeasuredParameter {
                key: "at",
                kind: MeasuredParameterKind::Choice {
                    options: &["ends", "sides", "within"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "How many of the two `ends` (across the long axis) or the two \
                     `sides` (along it) are obstructed, or how many obstacles stand \
                     `within` the rectangle, past none of its edges.",
                    "Wie viele der zwei Enden (`ends`, quer zur Längsachse) oder der \
                     zwei Seiten (`sides`, entlang) versperrt sind, oder wie viele \
                     Hindernisse innerhalb (`within`) des Rechtecks stehen.",
                ),
            },
            MeasuredParameter {
                key: "side_zone",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "The length of the central stretch of a side an obstacle must \
                     overlap, in metres; without it, the whole side.",
                    "Die Länge des mittleren Abschnitts einer Seite, den ein Hindernis \
                     überdecken muss, in Metern; ohne sie die ganze Seite.",
                ),
            },
        ],
        dimension: None,
        services: &[
            "plan-span",
            "proximity",
            "vertical-extent",
            "type-hierarchy",
        ],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            NO_LONG_AXIS,
            "an obstacle near it cannot be placed",
        ],
        label: &en_de("Obstruction count", "Anzahl Versperrungen"),
        help: &en_de(
            "How many ends or sides of the footprint's least-area rectangle obstacles \
             within reach obstruct, or how many stand within it, as `parking-bay` counts \
             them: a plain number, an interval when an obstacle only may obstruct.",
            "Wie viele Enden oder Seiten des flächenkleinsten Rechtecks Hindernisse in \
             Reichweite versperren oder wie viele darin stehen, wie `parking-bay` sie \
             zählt: eine Zahl, ein Intervall, wenn ein Hindernis nur versperren kann.",
        ),
    },
    MeasuredDescriptor {
        name: "offset",
        parameters: &[
            ALIGNMENT,
            ALIGNMENT_PATH,
            MeasuredParameter {
                key: "side",
                kind: MeasuredParameterKind::Choice {
                    options: &["left", "right"],
                },
                required: false,
                default: Some("left"),
                help: &en_de(
                    "The side counted positive, looking in the direction of travel.",
                    "Die positiv gezählte Seite, in Stationierungsrichtung gesehen.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[ALIGNMENT_UNREAD, OFF_RANGE, AMBIGUOUS_FOOT],
        label: &en_de("Offset", "Achsabstand"),
        help: &en_de(
            "The signed plan distance of the reference point from the alignment, \
             perpendicular to it at its foot.",
            "Der vorzeichenbehaftete Abstand des Bezugspunkts von der Achse im Lageplan, \
             senkrecht zu ihr an seinem Lotfußpunkt.",
        ),
    },
    MeasuredDescriptor {
        name: "opening_area",
        parameters: HOST_OPENINGS,
        dimension: Some(QuantityDimension::Area),
        services: &["relationship-selection", "body-facts"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            "an opening cannot be placed on the host's middle plane",
            "openings may overlap",
        ],
        label: &en_de("Opening area", "Öffnungsfläche"),
        help: &en_de(
            "The summed section areas of a host's openings on its middle plane, as \
             `opening-area` measures them against `gross_area − net_area`.",
            "Die summierten Schnittflächen der Öffnungen eines Wirts auf seiner \
             Mittelebene, wie `opening-area` sie mit `gross_area − net_area` vergleicht.",
        ),
    },
    MeasuredDescriptor {
        name: "opening_count",
        parameters: HOST_OPENINGS,
        dimension: None,
        services: &["relationship-selection", "body-facts"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            "an opening cannot be placed on the host's middle plane",
            "openings may overlap",
        ],
        label: &en_de("Voiding openings", "Durchbrechende Öffnungen"),
        help: &en_de(
            "How many of a host's openings take area from its middle plane, as \
             `opening_area` sums them: recesses short of it and openings below the \
             minimum are not counted.",
            "Wie viele Öffnungen eines Wirts seiner Mittelebene Fläche nehmen, wie \
             `opening_area` sie summiert: Nischen davor und Öffnungen unter dem Minimum \
             zählen nicht.",
        ),
    },
    MeasuredDescriptor {
        name: "opening_section_area",
        parameters: &[
            MeasuredParameter {
                key: "host_path",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from the opening to its one host.",
                    "Die Beziehungsschritte von der Öffnung zu ihrem einen Wirt.",
                ),
            },
            MeasuredParameter {
                key: "length_axis",
                kind: MeasuredParameterKind::Choice {
                    options: &["extrusion", "profile-x", "profile-y"],
                },
                required: false,
                default: Some("extrusion"),
                help: &en_de(
                    "The host's axis along its length.",
                    "Die Achse des Wirts entlang seiner Länge.",
                ),
            },
            MeasuredParameter {
                key: "height_axis",
                kind: MeasuredParameterKind::Choice {
                    options: &["extrusion", "profile-x", "profile-y"],
                },
                required: false,
                default: Some("profile-y"),
                help: &en_de(
                    "The host's axis along its height.",
                    "Die Achse des Wirts entlang seiner Höhe.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["relationship-selection", "body-facts"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[
            "the opening cannot be placed on its host's middle plane",
            "the opening voids several hosts",
        ],
        label: &en_de("Opening section area", "Öffnungsschnittfläche"),
        help: &en_de(
            "The area one opening takes from its host's middle plane: zero for a recess \
             stopping short of it; none when it voids no host.",
            "Die Fläche, die eine Öffnung der Mittelebene ihres Wirts nimmt: null für eine \
             Nische, die davor endet; keine, wenn sie keinen Wirt durchbricht.",
        ),
    },
    plain!(
        PERIMETER,
        Some(QuantityDimension::Length),
        &["plan-area"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Perimeter", "Umfang"),
        en_de(
            "The length of the footprint's boundary in plan, holes included.",
            "Die Länge des Grundrissrands, Löcher eingeschlossen."
        )
    ),
    MeasuredDescriptor {
        name: "plan_area",
        parameters: &[MeasuredParameter {
            key: "measure",
            kind: MeasuredParameterKind::Choice {
                options: &["footprint", "facade"],
            },
            required: false,
            default: Some("footprint"),
            help: &en_de(
                "The plan `footprint`, or the outward-facing `facade` surface.",
                "Der Grundriss (`footprint`) oder die nach außen weisende Fassadenfläche \
                 (`facade`).",
            ),
        }],
        dimension: Some(QuantityDimension::Area),
        services: &["plan-area", "facade-area"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the footprint is empty, so the object has no body",
        ],
        label: &en_de("Plan area", "Planfläche"),
        help: &en_de(
            "The object's own area as `plan-area` measures it: its footprint, which an \
             object with a body never has empty, or its facade area.",
            "Die eigene Fläche des Objekts, wie `plan-area` sie misst: sein Grundriss, \
             der bei einem Objekt mit Körper nie leer ist, oder seine Fassadenfläche.",
        ),
    },
    MeasuredDescriptor {
        name: "plan_coverage",
        parameters: &[
            selected(
                "candidates",
                true,
                &en_de(
                    "The candidates that may cover the footprint.",
                    "Die Kandidaten, die den Grundriss überdecken können.",
                ),
            ),
            MeasuredParameter {
                key: "minimum",
                kind: MeasuredParameterKind::Number { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The share searched for: a candidate surely covering it ends the search.",
                    "Der gesuchte Anteil: ein Kandidat, der ihn sicher überdeckt, beendet die Suche.",
                ),
            },
            TRAVERSAL[0],
            TRAVERSAL[1],
            TRAVERSAL[2],
            TRAVERSAL[3],
            TRAVERSAL[4],
        ],
        dimension: None,
        services: &["plan-area", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, "the subject has no plan footprint"],
        label: &en_de("Plan coverage", "Grundrissüberdeckung"),
        help: &en_de(
            "The share of the object's footprint within the candidate covering most of it, as `plan-coverage` searches the candidates the traversal reaches: a candidate surely covering the minimum ends the search, and a share or candidate that may reach it leaves the value reaching it at least.",
            "Der Anteil des Grundrisses des Objekts innerhalb des Kandidaten, der ihn am meisten überdeckt, wie `plan-coverage` die erreichten Kandidaten durchsucht: ein Kandidat, der das Minimum sicher überdeckt, beendet die Suche, und ein Anteil oder Kandidat, der es erreichen kann, lässt den Wert es mindestens erreichen.",
        ),
    },
    plain!(
        "plan_diameter",
        Some(QuantityDimension::Length),
        &["plan-span"],
        MeasuredExactness::Measured,
        &["the footprint has no point"],
        en_de("Longest plan diagonal", "Längste Diagonale im Grundriss"),
        en_de(
            "The longest distance between two points of the footprint.",
            "Der größte Abstand zweier Punkte des Grundrisses."
        )
    ),
    MeasuredDescriptor {
        name: "plan_overlap",
        parameters: &[
            objects(
                "with",
                true,
                &en_de(
                    "The source kinds overlapped, `,`-separated.",
                    "Die überlappten Quellarten, durch `,` getrennt.",
                ),
            ),
            MeasuredParameter {
                key: "measure",
                kind: MeasuredParameterKind::Choice {
                    options: &["largest", "total"],
                },
                required: false,
                default: Some("largest"),
                help: &en_de(
                    "The `largest` overlap with one of them, or their `total`.",
                    "Die größte Überlappung mit einem (`largest`) oder ihre Summe (`total`).",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["plan-area", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Plan overlap", "Grundrissüberlappung"),
        help: &en_de(
            "The footprint's overlap in plan with objects of the kinds named; with the footprint `area`, `plan-coverage`'s ratio.",
            "Die Überlappung des Grundrisses mit Objekten der genannten Arten; mit der Grundfläche `area` das Verhältnis von `plan-coverage`.",
        ),
    },
    MeasuredDescriptor {
        name: "prevailing_elevation",
        parameters: &[
            MeasuredParameter {
                key: "side",
                kind: MeasuredParameterKind::Choice {
                    options: &["bottom", "top"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The spaces' `bottom` or `top` elevation.",
                    "Die Unterkante (`bottom`) oder Oberkante (`top`) der Räume.",
                ),
            },
            MeasuredParameter {
                key: "spaces",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from a level to its spaces; the space's level is \
                     the one they reach back from it.",
                    "Die Beziehungsschritte von einem Geschoss zu seinen Räumen; das Geschoss \
                     des Raums ist das, das sie von ihm zurück erreichen.",
                ),
            },
            objects(
                "kinds",
                false,
                &en_de(
                    "The source kinds of the spaces compared, `,`-separated; without it, \
                     every object reached.",
                    "Die Quellarten der verglichenen Räume, durch `,` getrennt; ohne sie jedes \
                     erreichte Objekt.",
                ),
            ),
            metres(
                "tolerance",
                "0",
                &en_de(
                    "Within how many metres elevations count as one when the prevailing one \
                     is chosen.",
                    "Innerhalb wie vieler Meter Höhen bei der Wahl der vorherrschenden als \
                     eine zählen.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "no space of the level has an exact elevation, or the space reaches several \
             levels",
        ],
        label: &en_de("Prevailing elevation", "Vorherrschende Höhe"),
        help: &en_de(
            "The bottom or top elevation most spaces of the space's level share, the lowest \
             of equally common ones, from exact elevations only, as `level-spacing`'s \
             `space_elevation` compares them; none for a level with fewer than two spaces.",
            "Die Unter- oder Oberkante, die die meisten Räume des Geschosses des Raums \
             teilen, die niedrigste unter gleich häufigen, nur aus exakten Höhen, wie \
             `level-spacing` mit `space_elevation` sie vergleicht; keine für ein Geschoss mit \
             weniger als zwei Räumen.",
        ),
    },
    MeasuredDescriptor {
        name: "prevailing_rise",
        parameters: &[
            LEVEL_KINDS,
            LEVEL_ORDER,
            LEVEL_ANCHOR,
            LEVEL_PATH,
            LEVEL_ENDS[0],
            LEVEL_ENDS[1],
            LEVEL_ENDS[2],
            LEVEL_ENDS[3],
            metres(
                "tolerance",
                "0.001",
                &en_de(
                    "Within how many metres rises count as one when the prevailing one is \
                     chosen.",
                    "Innerhalb wie vieler Meter Steighöhen bei der Wahl der vorherrschenden \
                     als eine zählen.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: LEVEL_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[LEVEL_UNORDERED],
        label: &en_de("Prevailing rise", "Vorherrschende Geschosshöhe"),
        help: &en_de(
            "The level rise most of the anchor's checked levels share, the lowest of equally \
             common ones, from exact rises only, as `level-spacing`'s `consistent` compares \
             them; none with fewer than two rises or no exact one.",
            "Die Geschosshöhe, die die meisten geprüften Geschosse des Ankers teilen, die \
             niedrigste unter gleich häufigen, nur aus exakten Höhen, wie `level-spacing` mit \
             `consistent` sie vergleicht; keine bei weniger als zwei Höhen oder keiner \
             exakten.",
        ),
    },
    MeasuredDescriptor {
        name: "profile_dimension",
        parameters: &[MeasuredParameter {
            key: "name",
            kind: MeasuredParameterKind::Choice {
                options: &[
                    "width",
                    "depth",
                    "web_thickness",
                    "flange_thickness",
                    "thickness",
                    "wall_thickness",
                    "radius",
                    "girth",
                    "fillet_radius",
                    "semi_axis_1",
                    "semi_axis_2",
                    "top_width",
                    "top_offset",
                    "top_flange_thickness",
                    "top_fillet_radius",
                    "edge_radius",
                    "top_edge_radius",
                    "web_edge_radius",
                    "outer_fillet_radius",
                ],
            },
            required: true,
            default: None,
            help: &en_de(
                "The dimension, named alike for every profile family as `allowed-profile` \
                 names it.",
                "Die Abmessung, für jede Profilfamilie gleich benannt wie in \
                 `allowed-profile`.",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: &["property-resolution"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[PROFILE_UNREAD],
        label: &en_de("Profile dimension", "Profilabmessung"),
        help: &en_de(
            "A dimension of the member's swept profile, its parent's for a mirrored one; \
             none when its family has no such dimension or the source leaves it unset.",
            "Eine Abmessung des Profils des Bauteils, bei einem gespiegelten die seines \
             Ursprungs; keine, wenn die Familie sie nicht hat oder die Quelle sie nicht \
             angibt.",
        ),
    },
    MeasuredDescriptor {
        name: "profile_slope",
        parameters: &[MeasuredParameter {
            key: "name",
            kind: MeasuredParameterKind::Choice {
                options: &["flange_slope", "top_flange_slope", "leg_slope", "web_slope"],
            },
            required: true,
            default: None,
            help: &en_de(
                "The slope, as `allowed-profile` names it.",
                "Die Neigung, wie `allowed-profile` sie benennt.",
            ),
        }],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: &["property-resolution"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[PROFILE_UNREAD],
        label: &en_de("Profile slope", "Profilneigung"),
        help: &en_de(
            "A slope of the member's swept profile, an angle.",
            "Eine Neigung des Profils des Bauteils, ein Winkel.",
        ),
    },
    MeasuredDescriptor {
        name: "rails_over_surfaces",
        parameters: &[
            DEFECT_RAILS,
            DEFECT_REACH_ACROSS,
            DEFECT_REACH_ABOVE,
            kinds(
                "surfaces",
                &en_de(
                    "The source kinds of accessible surfaces.",
                    "Die Quellarten der barrierefreien Flächen.",
                ),
            ),
            MeasuredParameter {
                key: "of",
                kind: MeasuredParameterKind::Choice { options: &["ramp"] },
                required: false,
                default: Some("ramp"),
                help: &en_de("A ramp.", "Eine Rampe."),
            },
        ],
        dimension: None,
        services: DEFECTS,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, DEFECT_OPEN],
        label: &en_de(
            "Rails over accessible surfaces",
            "Handläufe über barrierefreien Flächen",
        ),
        help: &en_de(
            "How many times a rail of a ramp, or a rail joined to one, reaches over an accessible surface in plan.",
            "Wie oft ein Handlauf einer Rampe oder ein damit verbundener über eine barrierefreie Fläche im Grundriss reicht.",
        ),
    },
    MeasuredDescriptor {
        name: "ratio_area",
        parameters: &[
            stated(
                "property",
                &en_de(
                    "The area the object states, read instead of measuring one.",
                    "Die Fläche, die das Objekt angibt, statt eine zu messen.",
                ),
            ),
            MeasuredParameter {
                key: "measure",
                kind: MeasuredParameterKind::Choice {
                    options: &["footprint", "facade"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "The plan `footprint`, or the outward-facing `facade` surface.",
                    "Der Grundriss (`footprint`) oder die nach außen weisende \
                     Fassadenfläche (`facade`).",
                ),
            },
            MeasuredParameter {
                key: "otherwise",
                kind: MeasuredParameterKind::Choice {
                    options: &["footprint", "facade"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "The measure where `measure` is not given; the footprint where neither \
                     is.",
                    "Das Maß, wo `measure` nicht angegeben ist; der Grundriss, wo keines \
                     angegeben ist.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["plan-area", "facade-area", "property-resolution"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, "the property states no area"],
        label: &en_de("Area in a ratio", "Fläche in einem Verhältnis"),
        help: &en_de(
            "The object's area as `area-ratio` sums it: the area a property states, or \
             its footprint (an empty one counting zero) or facade area.",
            "Die Fläche des Objekts, wie `area-ratio` sie summiert: die angegebene Fläche \
             oder sein Grundriss (ein leerer zählt null) oder seine Fassadenfläche.",
        ),
    },
    MeasuredDescriptor {
        name: "rectangle_side",
        parameters: &[MeasuredParameter {
            key: "side",
            kind: MeasuredParameterKind::Choice {
                options: &["width", "length"],
            },
            required: true,
            default: None,
            help: &en_de(
                "The shorter side (`width`) or the longer (`length`).",
                "Die kürzere Seite (`width`) oder die längere (`length`).",
            ),
        }],
        dimension: Some(QuantityDimension::Length),
        services: &["plan-span"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the footprint has several least-area rectangles, or a tessellated one",
        ],
        label: &en_de("Rectangle side", "Rechteckseite"),
        help: &en_de(
            "A side of the footprint's least-area rectangle: its size along its own \
             axes, as `parking-bay` judges a bay's width and length.",
            "Eine Seite des flächenkleinsten Rechtecks um den Grundriss: seine Größe \
             entlang der eigenen Achsen, wie `parking-bay` Breite und Länge prüft.",
        ),
    },
    MeasuredDescriptor {
        name: "run_count",
        parameters: &[],
        dimension: None,
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT],
        label: &en_de("Ramp runs", "Rampenläufe"),
        help: &en_de(
            "How many sloped runs the walking-surface service measures of a ramp, as \
             `ramp-geometry` measures them before it judges any.",
            "Wie viele geneigte Läufe der Laufflächendienst an einer Rampe misst, wie \
             `ramp-geometry` sie misst, bevor es einen beurteilt.",
        ),
    },
    plain!(
        "section_area",
        Some(QuantityDimension::Area),
        &["property-resolution"],
        MeasuredExactness::Stated,
        &[PROFILE_UNREAD, "the family defines no section area here"],
        en_de("Section area", "Querschnittsfläche"),
        en_de(
            "The area of the member's profile: rectangles, hollow rectangles without \
             fillets, circles, hollow circles, ellipses and I-sections without slopes.",
            "Die Fläche des Profils: Rechtecke, Hohlrechtecke ohne Ausrundung, Kreise, \
             Hohlkreise, Ellipsen und I-Profile ohne Neigung."
        )
    ),
    MeasuredDescriptor {
        name: "section_modulus",
        parameters: &[MeasuredParameter {
            key: "axis",
            kind: MeasuredParameterKind::Choice {
                options: &["strong", "weak"],
            },
            required: false,
            default: Some("strong"),
            help: &en_de(
                "About the `strong` or `weak` axis.",
                "Um die starke (`strong`) oder schwache (`weak`) Achse.",
            ),
        }],
        dimension: Some(QuantityDimension::Volume),
        services: &["property-resolution"],
        exactness: MeasuredExactness::Stated,
        not_evaluated: &[PROFILE_UNREAD, "the family defines no section modulus here"],
        label: &en_de("Section modulus", "Widerstandsmoment"),
        help: &en_de(
            "The elastic section modulus of rectangular and circular profiles, solid or \
             hollow.",
            "Das elastische Widerstandsmoment rechteckiger und kreisförmiger Profile, voll \
             oder hohl.",
        ),
    },
    MeasuredDescriptor {
        name: "shelf_clear_height",
        parameters: &SHELVING,
        dimension: Some(QuantityDimension::Length),
        services: &["linear-quantity", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            SHELVING_UNMEASURED[0],
            SHELVING_UNMEASURED[1],
            SHELVING_UNMEASURED[2],
            "the service did not measure the clear height",
        ],
        label: &en_de("Shelving clear height", "Lichte Höhe für Regale"),
        help: &en_de(
            "The space's clear height the linear-quantity service measures with the \
             shelving, as `shelf-capacity` compares it with the shelving's top.",
            "Die lichte Höhe des Raums, die der Mengendienst mit den Regalen misst, wie \
             `shelf-capacity` sie mit der Oberkante der Regale vergleicht.",
        ),
    },
    MeasuredDescriptor {
        name: "shelf_length",
        parameters: &SHELVING,
        dimension: Some(QuantityDimension::Length),
        services: &["linear-quantity", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: SHELVING_UNMEASURED,
        label: &en_de("Shelf running metres", "Regalmeter"),
        help: &en_de(
            "The running metres of shelving the arrangement fits into the space, the \
             clearances of its doors and openings kept free, as `shelf-capacity` measures \
             them.",
            "Die laufenden Meter Regal, die die Anordnung im Raum unterbringt, die \
             Freiflächen seiner Türen und Öffnungen ausgespart, wie `shelf-capacity` sie \
             misst.",
        ),
    },
    MeasuredDescriptor {
        name: "sill_height",
        parameters: &[FLOOR_PATH, OVER_FLOORS],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, "a floor reached cannot be measured"],
        label: &en_de("Sill height", "Brüstungshöhe"),
        help: &en_de(
            "The object's bottom above the floors the path reaches, as `keyed-limit`'s \
             `sill-height` judges it per floor; none when it reaches none.",
            "Die Unterkante des Objekts über den erreichten Böden, wie `keyed-limit` \
             `sill-height` je Boden beurteilt; keine, wenn keiner erreicht wird.",
        ),
    },
    MeasuredDescriptor {
        name: SKEW,
        parameters: &[REFERENCE_PATH],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: RECTANGLES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_LONG_AXIS],
        label: &en_de("Skew", "Schiefe"),
        help: &en_de(
            "How far the footprint's long axis is from square to the long axes of the \
             objects a path reaches, in `[0, π/2]`: a pier's skew to the deck it carries.",
            "Wie weit die Längsachse des Grundrisses von rechtwinklig zu den Längsachsen \
             der erreichten Objekte abweicht, in `[0, π/2]`: die Schiefe eines Pfeilers.",
        ),
    },
    MeasuredDescriptor {
        name: SLOPE,
        parameters: &[FACE, FACING[0], FACING[1]],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_FACE, STANDS_VERTICAL, FACING_UNDECIDED],
        label: &en_de("Slope", "Neigung der Fläche"),
        help: &en_de(
            "The face's steepest gradient as an angle from the horizontal; a curved \
             or warped face gives the hull over its pieces. `convertSlope` turns it \
             into a ratio or percent.",
            "Das steilste Gefälle der Fläche als Winkel zur Waagerechten; eine \
             gekrümmte oder verwundene Fläche ergibt die Hülle über ihre Teile. \
             `convertSlope` rechnet es in ein Verhältnis oder Prozent um.",
        ),
    },
    MeasuredDescriptor {
        name: SLOPE_ALONG,
        parameters: &[
            MeasuredParameter {
                key: "direction",
                kind: PLAN_AXIS,
                required: true,
                default: None,
                help: &en_de(
                    "The plan direction: world `x` or `y`, or the object's own `own_x` \
                     or `own_y` in plan.",
                    "Die Grundrissrichtung: `x` oder `y` der Welt oder die eigene \
                     `own_x` oder `own_y` im Grundriss.",
                ),
            },
            FACE_UP_OR_DOWN,
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES_AND_FRAMES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_FACE, STANDS_VERTICAL],
        label: &en_de("Slope along", "Längsgefälle"),
        help: &en_de(
            "The face's gradient in a plan direction as a signed angle: rising along \
             it is positive.",
            "Das Gefälle der Fläche in einer Grundrissrichtung als Winkel mit \
             Vorzeichen: steigend ist positiv.",
        ),
    },
    MeasuredDescriptor {
        name: "stack_distance",
        parameters: &[
            MeasuredParameter {
                key: "measure",
                kind: MeasuredParameterKind::Choice {
                    options: &["top_to_top", "bottom_to_bottom", "top_to_bottom"],
                },
                required: true,
                default: None,
                help: &en_de(
                    "The rise from top to top, from bottom to bottom, or the clear gap from \
                     this slab's top to the next one's underside.",
                    "Der Anstieg von Oberkante zu Oberkante, von Unterkante zu Unterkante oder \
                     der lichte Abstand von der Oberkante dieser Platte zur Unterseite der \
                     nächsten.",
                ),
            },
            objects(
                "slabs",
                true,
                &en_de(
                    "The source kinds of the slabs stacked, `,`-separated.",
                    "Die Quellarten der gestapelten Platten, durch `,` getrennt.",
                ),
            ),
            MeasuredParameter {
                key: "ratio",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The least share of the smaller footprint two slabs overlap by to stack, \
                     above 0 and at most 1.",
                    "Der kleinste Anteil des kleineren Grundrisses, um den sich zwei Platten \
                     überlappen, um übereinander zu liegen, über 0 und höchstens 1.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "plan-area", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "whether two slabs stack, or which is the next one up, cannot be decided",
            "a slab of the kinds has no measurable extent",
        ],
        label: &en_de("Stack distance", "Stapelabstand"),
        help: &en_de(
            "The distance from the slab to the next slab up in its stack, as \
             `slab-stack-spacing` pairs and measures them; none where no slab stacks above.",
            "Der Abstand von der Platte zur nächsthöheren Platte ihres Stapels, wie \
             `slab-stack-spacing` sie paart und misst; keiner, wo keine Platte darüber liegt.",
        ),
    },
    MeasuredDescriptor {
        name: "stair_rise",
        parameters: &[
            MeasuredParameter {
                key: "stair",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from the stair to its parts.",
                    "Die Beziehungsschritte von der Treppe zu ihren Teilen.",
                ),
            },
            kinds(
                "flights",
                &en_de(
                    "The source kinds of its flights among its parts.",
                    "Die Quellarten ihrer Läufe unter ihren Teilen.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Length),
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT, "the path reaches no flight"],
        label: &en_de("Stair rise", "Treppenhöhe"),
        help: &en_de(
            "How far a whole stair rises, from its lowest flight's base to its highest \
             flight's top.",
            "Wie hoch eine ganze Treppe steigt, vom Fuß ihres untersten Laufs bis zum Kopf \
             ihres obersten.",
        ),
    },
    MeasuredDescriptor {
        name: "station",
        parameters: ALONG,
        dimension: Some(QuantityDimension::Length),
        services: ALIGNMENT_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[ALIGNMENT_UNREAD, OFF_RANGE, AMBIGUOUS_FOOT],
        label: &en_de("Station", "Station"),
        help: &en_de(
            "The station of the reference point's foot on the alignment: its plan distance from \
             the start, labelled through the station equations the alignment states.",
            "Die Station des Lotfußpunkts des Bezugspunkts auf der Achse: sein Abstand vom Anfang \
             im Lageplan, bezeichnet über die Stationsgleichungen der Achse.",
        ),
    },
    MeasuredDescriptor {
        name: "station_section_area",
        parameters: &[ALIGNMENT, ALIGNMENT_PATH, STATION],
        dimension: Some(QuantityDimension::Area),
        services: SECTION_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SECTION_UNREAD, STATION_OFF_RANGE, SECTION_BODY],
        label: &en_de(
            "Section area at a station",
            "Schnittfläche an einer Station",
        ),
        help: &en_de(
            "The area of the object's body cut by the vertical plane normal to the alignment \
             at the station; zero where the plane misses it.",
            "Die Fläche des Körpers des Objekts im lotrechten Schnitt normal zur Achse an der \
             Station; null, wo der Schnitt ihn verfehlt.",
        ),
    },
    MeasuredDescriptor {
        name: "station_section_thickness",
        parameters: &[
            ALIGNMENT,
            ALIGNMENT_PATH,
            STATION,
            MeasuredParameter {
                key: "direction",
                kind: MeasuredParameterKind::Choice {
                    options: &["lateral", "up"],
                },
                required: false,
                default: Some("up"),
                help: &en_de(
                    "Across the section horizontally (`lateral`) or vertically (`up`).",
                    "Quer durch den Schnitt waagerecht (`lateral`) oder lotrecht (`up`).",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: SECTION_SERVICES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SECTION_UNREAD, STATION_OFF_RANGE, SECTION_BODY],
        label: &en_de(
            "Section thickness at a station",
            "Schnittdicke an einer Station",
        ),
        help: &en_de(
            "How far the object's section at the station reaches across the direction: from \
             its lowest to its highest point vertically, or from its rightmost to its leftmost \
             horizontally; none where the plane misses it.",
            "Wie weit der Schnitt des Objekts an der Station in der Richtung reicht: lotrecht \
             vom tiefsten zum höchsten Punkt, waagerecht vom rechten zum linken Rand; keine, \
             wo der Schnitt ihn verfehlt.",
        ),
    },
    MeasuredDescriptor {
        name: "support_count",
        parameters: &[MeasuredParameter {
            key: "of",
            kind: MeasuredParameterKind::Choice {
                options: &["slabs", "roofs"],
            },
            required: false,
            default: Some("slabs"),
            help: &en_de(
                "Count the `slabs` or the `roofs`.",
                "Die Decken (`slabs`) oder die Dächer (`roofs`) zählen.",
            ),
        }],
        dimension: None,
        services: SPACE,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[SPACE_UNMEASURED],
        label: &en_de("Supporting elements", "Tragende Bauteile"),
        help: &en_de(
            "How many slabs or roofs the model has that can form a space's caps.",
            "Wie viele Decken oder Dächer das Modell hat, die Raumabdeckungen bilden \
             können.",
        ),
    },
    plain!(
        "swing_area",
        Some(QuantityDimension::Area),
        &["object-frame"],
        MeasuredExactness::Measured,
        &["the door states no leaves", "a leaf does not swing in plan"],
        en_de("Swing area", "Schwenkbereich"),
        en_de(
            "The plan area the leaves of a door or window sweep.",
            "Die Grundrissfläche, die die Flügel einer Tür oder eines Fensters überstreichen."
        )
    ),
    MeasuredDescriptor {
        name: "swings_into",
        parameters: &[MeasuredParameter {
            key: "path",
            kind: MeasuredParameterKind::Path,
            required: true,
            default: None,
            help: &en_de(
                "The relationship steps from the door to the spaces beside it.",
                "Die Beziehungsschritte von der Tür zu den Räumen daneben.",
            ),
        }],
        dimension: None,
        services: &["object-frame", "free-space", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "the door has no hinged leaf",
            "a space's side cannot be decided",
        ],
        label: &en_de("Swings into", "Schlägt auf in"),
        help: &en_de(
            "How many of the spaces the path reaches a door's leaves swing into.",
            "In wie viele der erreichten Räume die Flügel einer Tür aufschlagen.",
        ),
    },
    MeasuredDescriptor {
        name: THICKNESS,
        parameters: &[
            MeasuredParameter {
                key: "direction",
                kind: OWN_AXIS,
                required: false,
                default: None,
                help: &en_de(
                    "The axis the thickness is measured along; this or `face`.",
                    "Die Achse, entlang der die Dicke gemessen wird; dies oder `face`.",
                ),
            },
            MeasuredParameter {
                key: "face",
                kind: MeasuredParameterKind::Choice {
                    options: &["top", "bottom"],
                },
                required: false,
                default: None,
                help: &en_de(
                    "Measure square to this planar face; this or `direction`.",
                    "Rechtwinklig zu dieser ebenen Fläche messen; dies oder `direction`.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "object-frame"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the body is not closed, so it has no inside",
            "the face is not planar",
        ],
        label: &en_de("Thickness", "Dicke"),
        help: &en_de(
            "How thick the body is along a direction: an interval holding every local \
             thickness, so a tapered member spans its thinnest and thickest.",
            "Wie dick der Körper entlang einer Richtung ist: ein Intervall mit jeder \
             örtlichen Dicke, ein verjüngtes Bauteil reicht von dünnster bis dickster.",
        ),
    },
    MeasuredDescriptor {
        name: "threshold_step",
        parameters: &[
            FLOOR_PATH,
            stated(
                "threshold",
                &en_de(
                    "The threshold thickness the door states.",
                    "Die angegebene Schwellenhöhe der Tür.",
                ),
            ),
            OVER_FLOORS,
            objects(
                "ramps",
                false,
                &en_de(
                    "The source kinds of ramps, `,`-separated, subtypes included: a ramp \
                     within `ramp_reach` of the door and over a space is that side's floor, \
                     measured at its top.",
                    "Die Quellarten der Rampen, durch `,` getrennt, Untertypen \
                     eingeschlossen: eine Rampe innerhalb von `ramp_reach` der Tür über einem \
                     Raum ist der Boden dieser Seite, an ihrer Oberkante gemessen.",
                ),
            ),
            MeasuredParameter {
                key: "ramp_reach",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "With `ramps`, how far from the door in plan a ramp may lie, in metres.",
                    "Mit `ramps`, wie weit eine Rampe im Grundriss von der Tür liegen darf, \
                     in Metern.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection", "proximity"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the door states no threshold",
            "a floor reached cannot be measured",
        ],
        label: &en_de("Threshold step", "Schwellenstufe"),
        help: &en_de(
            "The step from each floor the path reaches to the door's bottom and \
             threshold, as `keyed-limit`'s `threshold-step` measures it: a ramp surely \
             near and over a space is its floor, one that only may be leaves both \
             possible.",
            "Die Stufe von jedem erreichten Boden zur Türunterkante mit Schwelle, wie \
             `keyed-limit` `threshold-step` misst: eine Rampe sicher nahe über einem Raum \
             ist sein Boden, eine nur mögliche lässt beide offen.",
        ),
    },
    plain!(
        MEASURED_TOP,
        Some(QuantityDimension::Length),
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Top elevation", "Oberkante"),
        en_de(
            "The elevation of the object's highest point.",
            "Die Höhe des höchsten Punkts des Objekts."
        )
    ),
    MeasuredDescriptor {
        name: "travel_distance",
        parameters: &[
            MeasuredParameter {
                key: "exits",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from the space to its exits.",
                    "Die Beziehungsschritte vom Raum zu seinen Ausgängen.",
                ),
            },
            kinds(
                "kinds",
                &en_de(
                    "The source kinds of the exits, `,`-separated, subtypes included.",
                    "Die Quellarten der Ausgänge, durch `,` getrennt, Untertypen eingeschlossen.",
                ),
            ),
            MeasuredParameter {
                key: "start",
                kind: MeasuredParameterKind::Choice {
                    options: &["farthest-point", "door"],
                },
                required: false,
                default: Some("farthest-point"),
                help: &en_de(
                    "Where the walk starts: every point of the space's walkable area, or each \
                     of its doors.",
                    "Wo der Weg beginnt: an jedem Punkt der begehbaren Fläche des Raums oder \
                     an jeder seiner Türen.",
                ),
            },
            MeasuredParameter {
                key: "doors",
                kind: MeasuredParameterKind::Path,
                required: false,
                default: None,
                help: &en_de(
                    "The relationship steps from the space to its doors, for `start=door`.",
                    "Die Beziehungsschritte vom Raum zu seinen Türen, für `start=door`.",
                ),
            },
            MeasuredParameter {
                key: "door_kinds",
                kind: MeasuredParameterKind::SourceKind,
                required: false,
                default: None,
                help: &en_de(
                    "The source kinds of the doors, `,`-separated.",
                    "Die Quellarten der Türen, durch `,` getrennt.",
                ),
            },
            MeasuredParameter {
                key: "walking_height",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The height a walker needs clear above the floor.",
                    "Die lichte Höhe, die ein Gehender über dem Boden braucht.",
                ),
            },
            MeasuredParameter {
                key: "walking_step",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: true,
                default: None,
                help: &en_de(
                    "The highest step a walker takes.",
                    "Die höchste Stufe, die ein Gehender nimmt.",
                ),
            },
        ],
        dimension: Some(QuantityDimension::Length),
        services: &[
            "metric-routing",
            "plan-span",
            "vertical-extent",
            "relationship-selection",
        ],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "an exit has no representative point",
            "the walk cannot be measured",
            "no door to start from",
        ],
        label: &en_de("Travel distance", "Fluchtweglänge"),
        help: &en_de(
            "The longest walk from a start to the nearest exit, as `escape-route` walks it \
             without multiplied sections; none where the space has no exit or a start \
             reaches none.",
            "Der längste Weg von einem Startpunkt zum nächsten Ausgang, wie `escape-route` \
             ihn geht, ohne vervielfachte Abschnitte; keiner, wo der Raum keinen Ausgang hat \
             oder ein Startpunkt keinen erreicht.",
        ),
    },
    plain!(
        "triangle_count",
        None,
        &["triangle-count"],
        MeasuredExactness::Measured,
        &[
            "the host could not mesh the body",
            "the host does not know the object",
        ],
        en_de("Triangle count", "Dreiecksanzahl"),
        en_de(
            "How many triangles the host's mesh of the body holds, as `triangle-count` \
             reads it: a whole number, cited as approximate where the mesh tessellates \
             curved faces, so the count depends on the host. A bodiless object counts none.",
            "Wie viele Dreiecke das Netz des Hosts für den Körper enthält, wie \
             `triangle-count` es liest: eine ganze Zahl, als angenähert belegt, wo das \
             Netz gekrümmte Flächen zerlegt und die Anzahl damit vom Host abhängt. Ein \
             Objekt ohne Körper zählt keine."
        )
    ),
    plain!(
        "unallocated_share",
        None,
        SPACE,
        MeasuredExactness::Measured,
        &[
            SPACE_UNMEASURED,
            "the storey's gross floor area is not measured"
        ],
        en_de("Unallocated share", "Unzugeordneter Anteil"),
        en_de(
            "The share of a storey's gross floor area no space covers, from 0 to 1.",
            "Der Anteil der Brutto-Geschossfläche, den kein Raum bedeckt, von 0 bis 1."
        )
    ),
    MeasuredDescriptor {
        name: "uncovered_area",
        parameters: &[
            objects(
                "by",
                true,
                &en_de(
                    "The source kinds that cover, `,`-separated.",
                    "Die bedeckenden Quellarten, durch `,` getrennt.",
                ),
            ),
            metres(
                "growth",
                "0",
                &en_de(
                    "How far each cover is grown in plan first, in metres.",
                    "Um wie viel jede Bedeckung im Grundriss zuvor vergrößert wird, in Metern.",
                ),
            ),
        ],
        dimension: Some(QuantityDimension::Area),
        services: &["plan-area", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY],
        label: &en_de("Uncovered area", "Unbedeckte Fläche"),
        help: &en_de(
            "The part of the footprint the objects of the kinds named, grown, leave uncovered in plan.",
            "Der Teil des Grundrisses, den die Objekte der genannten Arten, vergrößert, im Grundriss unbedeckt lassen.",
        ),
    },
    plain!(
        MEASURED_VOLUME,
        Some(QuantityDimension::Volume),
        &["proximity"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY, "the body is not closed"],
        en_de("Volume", "Volumen"),
        en_de(
            "The volume the object's body encloses.",
            "Das vom Körper des Objekts umschlossene Volumen."
        )
    ),
    MeasuredDescriptor {
        name: "walking_line_turns",
        parameters: &[],
        dimension: None,
        services: FLIGHT,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_FLIGHT],
        label: &en_de("Turning flight", "Gewendelter Lauf"),
        help: &en_de(
            "Whether a stair flight turns, so has winders: 1, or 0 for a straight flight.",
            "Ob ein Treppenlauf sich wendet und also Wendelstufen hat: 1, oder 0 für einen \
             geraden Lauf.",
        ),
    },
    MeasuredDescriptor {
        name: "well_gap",
        parameters: &[WELL_MEMBERS],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &["the path reaches no space"],
        label: &en_de("Largest gap in the well", "Größte Lücke im Schacht"),
        help: &en_de(
            "The largest vertical gap between consecutive stacked spaces, bottom to top.",
            "Die größte senkrechte Lücke zwischen aufeinanderfolgenden gestapelten Räumen, \
             von unten nach oben.",
        ),
    },
    MeasuredDescriptor {
        name: "well_height",
        parameters: &[WELL_MEMBERS],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &["the path reaches no space"],
        label: &en_de("Well height", "Schachthöhe"),
        help: &en_de(
            "From the lowest bottom to the highest top of the stacked spaces.",
            "Vom tiefsten Boden bis zur höchsten Oberkante der gestapelten Räume.",
        ),
    },
    MeasuredDescriptor {
        name: "well_section_area",
        parameters: &[WELL_MEMBERS],
        dimension: Some(QuantityDimension::Area),
        services: &["plan-span", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &["the path reaches no space"],
        label: &en_de("Well section area", "Schachtquerschnitt"),
        help: &en_de(
            "The area of the plan section the stacked spaces share.",
            "Die Fläche des Grundrissquerschnitts, den die gestapelten Räume teilen.",
        ),
    },
    MeasuredDescriptor {
        name: "well_section_width",
        parameters: &[WELL_MEMBERS],
        dimension: Some(QuantityDimension::Length),
        services: &["plan-span", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "the path reaches no space",
            "the section's rectangle is tied",
        ],
        label: &en_de("Well section width", "Schachtbreite"),
        help: &en_de(
            "The short side of the shared section's least-area rectangle; none for an empty \
             section.",
            "Die kurze Seite des kleinsten umschließenden Rechtecks des geteilten \
             Querschnitts; keine bei leerem Querschnitt.",
        ),
    },
    plain!(
        MEASURED_X,
        Some(QuantityDimension::Length),
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin x", "Ursprung x"),
        en_de(
            "The world x coordinate of the object's placement origin.",
            "Die x-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
    plain!(
        MEASURED_Y,
        Some(QuantityDimension::Length),
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin y", "Ursprung y"),
        en_de(
            "The world y coordinate of the object's placement origin.",
            "Die y-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
    plain!(
        MEASURED_Z,
        Some(QuantityDimension::Length),
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin z", "Ursprung z"),
        en_de(
            "The world z coordinate of the object's placement origin.",
            "Die z-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
];
