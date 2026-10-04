//! Every measured value's descriptor, sorted by name.

use super::{
    ANGLE_TO, BEARING, CROSS_FALL, EXTENT, GRADIENT_DIRECTION, INCLINATION, LENGTH, LocalizedText,
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
        dimension: QuantityDimension::PlaneAngle,
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
        QuantityDimension::Area,
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
        dimension: QuantityDimension::PlaneAngle,
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
        QuantityDimension::Length,
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
        dimension: QuantityDimension::Length,
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
        dimension: QuantityDimension::Area,
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
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::Length,
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
        QuantityDimension::Length,
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
        QuantityDimension::Length,
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
        QuantityDimension::Length,
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
        name: GRADIENT_DIRECTION,
        parameters: &[FACE],
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::Length,
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
        QuantityDimension::Length,
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
    plain!(
        PERIMETER,
        QuantityDimension::Length,
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
        name: SKEW,
        parameters: &[REFERENCE_PATH],
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::PlaneAngle,
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
        dimension: QuantityDimension::Length,
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
    plain!(
        MEASURED_TOP,
        QuantityDimension::Length,
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
        MEASURED_VOLUME,
        QuantityDimension::Volume,
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
        QuantityDimension::Length,
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
        QuantityDimension::Length,
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
        QuantityDimension::Length,
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
