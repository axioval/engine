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

const fn en_de(en: &'static str, de: &'static str) -> [LocalizedText; 2] {
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

const OWN_AXIS: MeasuredParameterKind = MeasuredParameterKind::Choice {
    options: &["own_x", "own_y", "own_z", "x", "y", "z"],
};

const PLAN_AXIS: MeasuredParameterKind = MeasuredParameterKind::Choice {
    options: &["x", "y", "own_x", "own_y"],
};
const NO_GEOMETRY: &str = "the object has no body the service can measure";

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
