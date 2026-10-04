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

pub(super) const fn en_de(en: &'static str, de: &'static str) -> [LocalizedText; 2] {
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
const NO_FACE: &str = "the body has no such face, or it cannot be told apart";
const STANDS_VERTICAL: &str = "a piece of the face stands vertical, so it has no gradient";

const FACE: MeasuredParameter = MeasuredParameter {
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
const NO_GEOMETRY: &str = "the object has no body the service can measure";

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

/// Every measured value, sorted by name.
pub static MEASURED_VALUES: &[MeasuredDescriptor] = &[
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
        ],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES_AND_FRAMES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_FACE, STANDS_VERTICAL],
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
        parameters: &[],
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
        parameters: &[FACE],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            NO_FACE,
            STANDS_VERTICAL,
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
        name: "opening_area",
        parameters: &[
            MeasuredParameter {
                key: "path",
                kind: MeasuredParameterKind::Path,
                required: true,
                default: None,
                help: &en_de(
                    "The relationship steps from the host to its openings.",
                    "Die Beziehungsschritte vom Wirt zu seinen Öffnungen.",
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
            MeasuredParameter {
                key: "minimum",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: None,
                help: &en_de(
                    "Openings smaller than this many square metres are not counted.",
                    "Öffnungen kleiner als so viele Quadratmeter werden nicht gezählt.",
                ),
            },
        ],
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
        parameters: &[FACE],
        dimension: Some(QuantityDimension::PlaneAngle),
        services: FACES,
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[NO_GEOMETRY, NO_FACE, STANDS_VERTICAL],
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
            FACE,
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
        ],
        dimension: Some(QuantityDimension::Length),
        services: &["vertical-extent", "relationship-selection"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "the door states no threshold",
            "a floor reached cannot be measured",
        ],
        label: &en_de("Threshold step", "Schwellenstufe"),
        help: &en_de(
            "The step from each floor the path reaches to the door's bottom and \
             threshold, as `keyed-limit`'s `threshold-step` measures it without ramps.",
            "Die Stufe von jedem erreichten Boden zur Türunterkante mit Schwelle, wie \
             `keyed-limit` `threshold-step` ohne Rampen misst.",
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
