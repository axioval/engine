//! Every measured value's descriptor, sorted by name.

use super::{
    CROSS_FALL, GRADIENT_DIRECTION, INCLINATION, LocalizedText, MeasuredDescriptor,
    MeasuredExactness, MeasuredParameter, MeasuredParameterKind, SLOPE, SLOPE_ALONG,
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
