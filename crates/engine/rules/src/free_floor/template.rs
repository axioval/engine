//! `free-floor-circle` and `free-floor-rectangle` as templates: the
//! free-floor search's answer for each space (`free_floor_fit`), a proven
//! absence a finding, a fit a pass and a search the selections leave open
//! not evaluated.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, ItemText, Items, Judge,
    Refusals, Service, Services, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// Every parameter of the declaration, under its own name.
macro_rules! options {
    () => {
        "obstacles=@obstacles;band_from_metres=@band_from_metres;\
         band_to_metres=@band_to_metres;merge_path=@merge_path;\
         subtract_door_swings=@subtract_door_swings;entrance_path_width=@entrance_path_width;\
         entrance_tolerance_metres=@entrance_tolerance_metres;access_path=@access_path;\
         door_selector=@door_selector;opening_selector=@opening_selector;\
         space_selector=@space_selector"
    };
}

const CIRCLE: &str = concat!(
    "free_floor_fit;shape=circle;diameter_metres=@diameter_metres;height_metres=@height_metres;",
    options!()
);

const RECTANGLE: &str = concat!(
    "free_floor_fit;shape=rectangle;width_metres=@width_metres;length_metres=@length_metres;\
     height_metres=@height_metres;orientation=@orientation;",
    options!()
);

/// The circle's parameter descriptor.
pub(crate) fn circle_parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::required("diameter_metres", ParameterType::Number),
        ParameterDescriptor::required("height_metres", ParameterType::Number),
    ];
    parameters.extend(super::parameters());
    parameters
}

/// The rectangle's parameter descriptor.
pub(crate) fn rectangle_parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::required("width_metres", ParameterType::Number),
        ParameterDescriptor::required("length_metres", ParameterType::Number),
        ParameterDescriptor::required("height_metres", ParameterType::Number),
        // Optional in the signature so that a rule without it is reported
        // per object as an invalid declaration, not rejected with its whole
        // package.
        ParameterDescriptor::optional("orientation", ParameterType::String),
    ];
    parameters.extend(super::parameters());
    parameters
}

/// What a proven absence measured of the floor, as words (#391): the
/// largest shape of the declared proportions (`largest` the circle's or
/// the rectangle's words, placed after the code where nothing precedes
/// it), the free floor's area and the obstacles that may also take floor.
fn measured_texts(largest: [&'static str; 2]) -> Vec<ItemText> {
    // Nothing precedes: no largest shape, and the shape reached.
    let first = || {
        vec![
            When::Null { field: "largest" },
            When::Null {
                field: "largest_width",
            },
            When::Field {
                field: "unreached",
                value: false,
            },
        ]
    };
    vec![
        ItemText {
            name: "largest_note",
            when: vec![
                When::Null { field: "largest" },
                When::Null {
                    field: "largest_width",
                },
            ],
            text: "",
        },
        ItemText {
            name: "largest_note",
            when: vec![When::Null { field: "centre_x" }],
            text: largest[0],
        },
        ItemText {
            name: "largest_note",
            when: Vec::new(),
            text: largest[1],
        },
        ItemText {
            name: "area_note",
            when: vec![When::Null { field: "free_area" }],
            text: "",
        },
        ItemText {
            name: "area_note",
            when: first(),
            text: ": the free floor is {free_area:area} m²",
        },
        ItemText {
            name: "area_note",
            when: Vec::new(),
            text: "; the free floor is {free_area:area} m²",
        },
        ItemText {
            name: "possible_note",
            when: vec![When::Empty {
                field: "possible_obstacles",
            }],
            text: "",
        },
        ItemText {
            name: "possible_note",
            when: {
                let mut when = first();
                when.push(When::Null { field: "free_area" });
                when
            },
            text: ": perhaps also taken by {possible_obstacles}",
        },
        ItemText {
            name: "possible_note",
            when: Vec::new(),
            text: "; perhaps also taken by {possible_obstacles}",
        },
    ]
}

/// The search's answer judged: no placement a finding worded `fail`.
fn fit(list: &'static str, fail: &'static str, largest: [&'static str; 2]) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list,
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(ItemTest {
                applies: Applies::default(),
                when: Vec::new(),
                judge: Judge::Truth {
                    value: "fits",
                    finding: false,
                },
                fail,
                undecided: "{why}",
                effects: Vec::new(),
                then: None,
                otherwise: None,
                straddled: None,
                related: Some("related"),
            }))],
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "unreached_note",
                    when: vec![When::Field {
                        field: "unreached",
                        value: true,
                    }],
                    text: ": the shape fits only where no path {entrance_path_width:length} \
                           wide from an entrance reaches it",
                },
                ItemText {
                    name: "unreached_note",
                    when: Vec::new(),
                    text: "",
                },
            ]
            .into_iter()
            .chain(measured_texts(largest))
            .collect(),
            merged: false,
            at: None,
            once: false,
            combined: None,
            reason: None,
            joined: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// One free-floor capability as a template: `list` the search it judges,
/// `fail` its finding's message.
fn template(
    (id, name): (&'static str, &'static str),
    parameters: Vec<ParameterDescriptor>,
    list: &'static str,
    (fail, largest): (&'static str, [&'static str; 2]),
) -> Template {
    Template {
        id,
        parameters,
        grades: false,
        name,
        refusals: Refusals::Objects,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: list,
        }],
        services: Some(Services {
            needs: vec![Service::FreeSpace],
            message: "free-space service is not registered",
            only: None,
            whole: false,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the free floor is searched".into()),
                },
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Within {
                value: "judged",
                minimum: None,
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: None,
            checks: vec![fit(list, fail, largest)],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}

/// `free-floor-circle`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn circle() -> Template {
    template(
        ("axioval:capability.free-floor-circle", "free-floor-circle"),
        circle_parameters(),
        CIRCLE,
        (
            "NO_FREE_FLOOR_SPACE_FOR_CIRCLE{unreached_note}{largest_note}{area_note}\
             {possible_note}",
            [
                ": the largest free circle is {largest:length} across; required \
                 {diameter_metres:length}",
                ": the largest free circle is {largest:length} across, centred at \
                 ({centre_x:length}, {centre_y:length}, {centre_z:length}); required \
                 {diameter_metres:length}",
            ],
        ),
    )
}

/// `free-floor-rectangle`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn rectangle() -> Template {
    template(
        (
            "axioval:capability.free-floor-rectangle",
            "free-floor-rectangle",
        ),
        rectangle_parameters(),
        RECTANGLE,
        (
            "NO_FREE_FLOOR_SPACE_FOR_RECTANGLE{unreached_note}{largest_note}{area_note}\
             {possible_note}",
            [
                ": the largest free rectangle of its proportions is \
                 {largest_width:length} by {largest_length:length}; required \
                 {width_metres:length} by {length_metres:length}",
                ": the largest free rectangle of its proportions is \
                 {largest_width:length} by {largest_length:length}, centred at \
                 ({centre_x:length}, {centre_y:length}, {centre_z:length}); required \
                 {width_metres:length} by {length_metres:length}",
            ],
        ),
    )
}
