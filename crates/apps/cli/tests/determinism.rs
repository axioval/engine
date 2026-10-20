//! The determinism gate (#441, ADR 0006): a check's result is the same,
//! byte for byte, whatever order its rules are evaluated in, on however
//! many threads, and whether or not anything built for one rule is kept
//! for another.
//!
//! Each variant runs the real binary, `axioval check --rule-status`, with
//! the hidden diagnostics `--rule-order` (`plan`, `reversed`,
//! `shuffled:SEED`: the order wherever the rules' dependencies leave it
//! free), `--no-reuse` (a derivation store that keeps nothing, and
//! geometry services that keep no placement scene or footprint) and
//! `--keep-derivations` (a store releasing what it keeps past a bound,
//! #440) beside `--threads`. On more than one thread a check evaluates its
//! independent rules at once (#406); `--serial-rules` evaluates them one
//! after another, each on the threads, and both run here. Every variant's
//! standard output must be the first's. The
//! harness knows nothing of what a variant changes: a switch added later
//! (a memory budget that makes evaluation wait, a store on disk) is one
//! more row of [`VARIANTS`].
//!
//! The same orders, threads and reuse run again under an expression
//! evaluation budget small enough to run out ([`BUDGETED`], the hidden
//! `--evaluation-budget`): each rule's share runs out part way and the
//! backstop refuses the rules after the first that spent the run's size,
//! alike in every variant (#447).
//!
//! The rulesets are the parity cases' that share the most between rules:
//! `floors` (free-floor circle and rectangle searches, clearances and
//! circulation over the same spaces), `counts` (relationship walks),
//! `coverage` and `spans` (measured values and selections shared between
//! rules) and `judges` (shared selections and property reads). They run
//! over a generated building in every gate, and over the pinned public
//! models where `AXIOVAL_PARITY_MODELS` names them (the gate's `test`
//! section and CI's `parity` job).
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../fixtures/parity");

/// The variants every check runs in, the first the reference: the plan's
/// order on one thread, keeping what it builds.
const VARIANTS: &[(&str, &[&str])] = &[
    ("plan, 1 thread", &["--threads", "1"]),
    (
        "reversed, 1 thread",
        &["--threads", "1", "--rule-order", "reversed"],
    ),
    ("plan, 4 threads, rules at once", &["--threads", "4"]),
    (
        "plan, 4 threads, rules one after another",
        &["--threads", "4", "--serial-rules"],
    ),
    // More threads than most cases have rules ready at once, so rules
    // wait for the rules they read while others run (#406).
    (
        "reversed, 8 threads, rules at once",
        &["--threads", "8", "--rule-order", "reversed"],
    ),
    (
        "shuffled (seed 1), 4 threads",
        &["--threads", "4", "--rule-order", "shuffled:1"],
    ),
    (
        "reversed, 4 threads, no reuse",
        &["--threads", "4", "--rule-order", "reversed", "--no-reuse"],
    ),
    (
        "shuffled (seed 2), 1 thread, no reuse",
        &["--threads", "1", "--rule-order", "shuffled:2", "--no-reuse"],
    ),
    // A memory budget smaller than the steps running at once on four
    // threads, so they wait for each other (#407): the `coverage` case's
    // overlays wait on the public architecture models, whose largest step
    // is estimated at 37 MiB. Above every one step's estimate, it refuses
    // none.
    (
        "plan, 4 threads, a small memory budget",
        &["--threads", "4", "--memory-budget", "64M"],
    ),
    // A store keeping one byte (#440): every value is released as soon as
    // another lands and built again when read again, scenes and indexes
    // between the subjects of one rule and between rules, and each read
    // is charged as if built.
    (
        "shuffled (seed 3), 4 threads, keeping one byte of derivations",
        &[
            "--threads",
            "4",
            "--rule-order",
            "shuffled:3",
            "--keep-derivations",
            "1",
        ],
    ),
];

/// An expression evaluation budget most rules of the cases spend part
/// way, and the variants run under it, the first the reference.
const BUDGETED: &[(&str, &[&str])] = &[
    (
        "plan, 1 thread, budget 10",
        &["--threads", "1", "--evaluation-budget", "10"],
    ),
    (
        "reversed, 1 thread, budget 10",
        &[
            "--threads",
            "1",
            "--rule-order",
            "reversed",
            "--evaluation-budget",
            "10",
        ],
    ),
    (
        "shuffled (seed 1), 4 threads, budget 10",
        &[
            "--threads",
            "4",
            "--rule-order",
            "shuffled:1",
            "--evaluation-budget",
            "10",
        ],
    ),
    (
        "plan, 4 threads, rules one after another, budget 10",
        &[
            "--threads",
            "4",
            "--serial-rules",
            "--evaluation-budget",
            "10",
        ],
    ),
    (
        "plan, 8 threads, rules at once, budget 10",
        &["--threads", "8", "--evaluation-budget", "10"],
    ),
    (
        "reversed, 4 threads, no reuse, budget 10",
        &[
            "--threads",
            "4",
            "--rule-order",
            "reversed",
            "--no-reuse",
            "--evaluation-budget",
            "10",
        ],
    ),
];

/// The parity cases whose rulesets the gate runs.
const CASES: &[&str] = &["floors", "counts", "coverage", "spans", "judges"];

/// How many checks run at once: each on up to four threads.
const AT_ONCE: usize = 4;

/// The rules of the cases the generated building leaves out, for time:
/// the circulation searches but the first, each a second or more per
/// variant in a debug build, and each reading only the scenes the rules
/// kept beside it read too.
const FIXTURE_LEFT_OUT: &[&str] = &["lc-ends", "lc-entrances", "lc-link", "lc-passing"];

/// One check of the gate: a parity case's packages over a model.
struct Check {
    case: &'static str,
    /// The model's name and path.
    model: (String, PathBuf),
    /// The case's ruleset, or the gate's copy leaving rules out.
    ruleset: PathBuf,
}

/// The ruleset of the parity case `case`.
fn case_ruleset(case: &str) -> PathBuf {
    Path::new(FIXTURES)
        .join("cases")
        .join(case)
        .join("ruleset.json")
}

/// The standard output of `check` over the check's model with the
/// packages of its case, meshed unless the case says otherwise, with
/// `args`.
fn result(check: &Check, args: &[&str]) -> Vec<u8> {
    run(check, args).stdout
}

/// [`result`], with what `check` printed on standard error.
fn run(check: &Check, args: &[&str]) -> std::process::Output {
    let model = &check.model.1;
    let case = Path::new(FIXTURES).join("cases").join(check.case);
    let parity: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(case.join("parity.json")).expect("the case's parity.json"),
    )
    .expect("parity.json is JSON");
    let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
    command.arg("check");
    if parity["geometry"].as_bool().unwrap_or(true) {
        command.arg("--geometry");
    }
    let output = command
        .arg("--rule-status")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(case.join("definitions.json"))
        .arg("--ruleset")
        .arg(&check.ruleset)
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1790416800")
        .output()
        .unwrap();
    // 0: nothing found, 3: findings, 4: something not evaluated.
    assert!(
        matches!(output.status.code(), Some(0 | 3 | 4)),
        "{} over {} with {args:?}: {}\n{}",
        case.display(),
        model.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Where two results first differ: the line of each, as JSON is printed.
fn first_difference(expected: &[u8], actual: &[u8]) -> String {
    let pretty = |bytes: &[u8]| {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
    };
    let (expected, actual) = (pretty(expected), pretty(actual));
    let mut lines = expected.lines().zip(actual.lines()).enumerate();
    match lines.find(|(_, (a, b))| a != b) {
        Some((line, (a, b))) => format!("line {}: `{a}` against `{b}`", line + 1),
        None => format!(
            "{} lines against {}",
            expected.lines().count(),
            actual.lines().count()
        ),
    }
}

/// Runs every check in every one of `variants`, at most [`AT_ONCE`] at a
/// time; the differences from each check's first variant, one line each,
/// and each check's first output.
fn differences(checks: &[Check], variants: &[(&str, &[&str])]) -> (Vec<String>, Vec<Vec<u8>>) {
    let runs: Vec<(usize, usize)> = (0..checks.len())
        .flat_map(|check| (0..variants.len()).map(move |variant| (check, variant)))
        .collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Vec<Option<Vec<u8>>>>> =
        Mutex::new(checks.iter().map(|_| vec![None; variants.len()]).collect());
    std::thread::scope(|scope| {
        for _ in 0..AT_ONCE {
            scope.spawn(|| {
                while let Some(&(check, variant)) = runs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let output = result(&checks[check], variants[variant].1);
                    results.lock().unwrap()[check][variant] = Some(output);
                }
            });
        }
    });
    let results = results.into_inner().unwrap();
    let mut found = Vec::new();
    let mut references = Vec::new();
    for (check, outputs) in checks.iter().zip(results) {
        let mut outputs: Vec<Vec<u8>> = outputs
            .into_iter()
            .map(|output| output.expect("every variant ran"))
            .collect();
        let (reference, expected) = (variants[0].0, &outputs[0]);
        for ((variant, _), actual) in variants.iter().zip(&outputs).skip(1) {
            if actual != expected {
                found.push(format!(
                    "{} over {}: {variant} differs from {reference}: {}",
                    check.case,
                    check.model.0,
                    first_difference(expected, actual)
                ));
            }
        }
        references.push(outputs.swap_remove(0));
    }
    (found, references)
}

/// How many rules of `output` the evaluation budget left something of
/// open, and how many the backstop refused whole.
fn budget_refusals(output: &[u8]) -> (usize, usize) {
    let output: serde_json::Value = serde_json::from_slice(output).unwrap();
    let limited: Vec<&serde_json::Value> = output["report"]["not_evaluated"]
        .as_array()
        .map(|outcomes| {
            outcomes
                .iter()
                .filter(|outcome| outcome["reason"] == "resource_limit")
                .collect()
        })
        .unwrap_or_default();
    let refused = |whole: bool| {
        let mut rules: Vec<&str> = limited
            .iter()
            .filter(|outcome| {
                outcome["message"]
                    .as_str()
                    .is_some_and(|message| message.starts_with("the rules before it"))
                    == whole
            })
            .filter_map(|outcome| outcome["rule_id"].as_str())
            .collect();
        rules.sort_unstable();
        rules.dedup();
        rules.len()
    };
    (refused(false), refused(true))
}

/// A box `[length, width, depth]` centred at `(x, y)`, its bottom at `z`,
/// as instances `#first` to `#first + 9`; `PL`, `REP` and `GID` in
/// `product` become its placement, shape and identity.
fn placed_box(
    first: u32,
    [x, y, z]: [f64; 3],
    [length, width, depth]: [f64; 3],
    product: &str,
) -> String {
    let [
        origin,
        frame,
        placement,
        p,
        pos,
        profile,
        solid,
        shape,
        definition,
        object,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(|offset| first + offset);
    format!(
        "#{origin}=IFCCARTESIANPOINT((0.,0.,{z:.2}));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{p}=IFCCARTESIANPOINT(({x:.2},{y:.2}));\n\
         #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:.2},{width:.2});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:.2});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}")),
    )
}

/// A storey of `rows` rows of three rooms, each 4 x 3 m and 2.5 m high,
/// between walls 0.2 m thick on a slab: every room holds a table, every
/// other one a wardrobe along its north wall, and every third a cabinet
/// in its south-west corner, so the free-floor circle and rectangle rules
/// search each room's floor (a shape fits some rooms and not others). The
/// walls, slab and furniture are contained in the storey, its rooms
/// aggregated into it, so relationship walks reach them.
fn building(rows: u32) -> String {
    let space = "IFCSPACE('GID',$,'Room',$,$,PL,REP,$,.ELEMENT.,$,$)";
    let wall = "IFCWALL('GID',$,'Wall',$,$,PL,REP,$,.STANDARD.)";
    let slab = "IFCSLAB('GID',$,'Floor',$,$,PL,REP,$,.FLOOR.)";
    let furniture = "IFCFURNITURE('GID',$,'Furniture',$,$,PL,REP,$,$)";
    let (columns, length, width) = (3_u32, 4.2, 3.2);
    let (span_x, span_y) = (f64::from(columns) * length, f64::from(rows) * width);
    let mut data = placed_box(
        100,
        [span_x / 2.0, span_y / 2.0, -0.2],
        [span_x + 0.2, span_y + 0.2, 0.2],
        slab,
    );
    let mut spaces = Vec::new();
    let mut contained = vec![109];
    let mut next = 200;
    let mut add = |data: &mut String, centre: [f64; 3], size: [f64; 3], product: &str| {
        data.push_str(&placed_box(next, centre, size, product));
        next += 10;
        next - 1
    };
    // Walls along x (south of each row, and north of the last) and along y.
    for row in 0..=rows {
        let y = f64::from(row) * width;
        contained.push(add(
            &mut data,
            [span_x / 2.0, y, 0.0],
            [span_x + 0.2, 0.2, 2.7],
            wall,
        ));
    }
    for column in 0..=columns {
        let x = f64::from(column) * length;
        contained.push(add(
            &mut data,
            [x, span_y / 2.0, 0.0],
            [0.2, span_y - 0.2, 2.7],
            wall,
        ));
    }
    let mut room = 0;
    for row in 0..rows {
        for column in 0..columns {
            room += 1;
            let (x, y) = (f64::from(column) * length, f64::from(row) * width);
            spaces.push(add(
                &mut data,
                [x + length / 2.0, y + width / 2.0, 0.0],
                [length - 0.2, width - 0.2, 2.5],
                space,
            ));
            contained.push(add(
                &mut data,
                [x + 2.6, y + 1.4, 0.0],
                [1.2, 0.8, 0.75],
                furniture,
            ));
            if room % 2 == 0 {
                contained.push(add(
                    &mut data,
                    [x + 2.1, y + width - 0.45, 0.0],
                    [2.0, 0.6, 2.0],
                    furniture,
                ));
            }
            if room % 3 == 0 {
                contained.push(add(
                    &mut data,
                    [x + 0.5, y + 0.5, 0.0],
                    [0.6, 0.6, 0.9],
                    furniture,
                ));
            }
        }
    }
    let list = |ids: &[u32]| {
        ids.iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);\n\
         #8=IFCUNITASSIGNMENT((#6,#7));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#8);\n\
         #10=IFCBUILDING('0000000000000000000010',$,'B',$,$,#3,$,$,.ELEMENT.,$,$,$);\n\
         #11=IFCBUILDINGSTOREY('0000000000000000000011',$,'0',$,$,#3,$,$,.ELEMENT.,0.);\n\
         #12=IFCRELAGGREGATES('0000000000000000000012',$,$,$,#9,(#10));\n\
         #13=IFCRELAGGREGATES('0000000000000000000013',$,$,$,#10,(#11));\n\
         #14=IFCRELAGGREGATES('0000000000000000000014',$,$,$,#11,({}));\n\
         #15=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000015',$,$,$,({}),#11);\n\
         {data}ENDSEC;\nEND-ISO-10303-21;\n",
        list(&spaces),
        list(&contained),
    )
}

/// Every variant reports what the first does over a generated building:
/// the gate's fixture, run in every `./scripts/check.sh test`.
#[test]
fn a_generated_building_is_judged_alike_whatever_the_order_threads_and_reuse() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("determinism");
    std::fs::create_dir_all(&directory).unwrap();
    let model = directory.join("building.ifc");
    std::fs::write(&model, building(1)).unwrap();
    let checks: Vec<Check> = CASES
        .iter()
        .map(|case| {
            let mut ruleset: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(case_ruleset(case)).unwrap())
                    .unwrap();
            ruleset["root"]["rules"]
                .as_array_mut()
                .expect("the case's rules")
                .retain(|rule| !FIXTURE_LEFT_OUT.contains(&rule["id"].as_str().unwrap_or("")));
            let path = directory.join(format!("{case}.ruleset.json"));
            std::fs::write(&path, ruleset.to_string()).unwrap();
            Check {
                case,
                model: ("building.ifc".to_owned(), model.clone()),
                ruleset: path,
            }
        })
        .collect();
    // The building is searched, not refused: some free-floor rules find a
    // room too small for their shapes, and others pass every room.
    let floors: serde_json::Value =
        serde_json::from_slice(&result(&checks[0], VARIANTS[0].1)).unwrap();
    let statuses: Vec<(&str, &str)> = floors["report"]["rules"]
        .as_array()
        .expect("rule statuses")
        .iter()
        .map(|rule| {
            (
                rule["rule_id"].as_str().unwrap_or(""),
                rule["status"].as_str().unwrap_or(""),
            )
        })
        .filter(|(rule, _)| rule.starts_with("ff-"))
        .collect();
    for status in ["passed", "failed"] {
        assert!(
            statuses.iter().any(|(_, decided)| *decided == status),
            "no free-floor rule {status}: {statuses:?}"
        );
    }
    let (differences, _) = differences(&checks, VARIANTS);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    // Under a budget that runs out, rules are refused part way and whole,
    // alike in every variant.
    let (differences, references) = differences_budgeted(&checks);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    let (part_way, whole) = references
        .iter()
        .map(|output| budget_refusals(output))
        .fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
    assert!(
        part_way > 0 && whole > 0,
        "{part_way} part way, {whole} whole"
    );
}

/// The `--stats` lines of a check's standard error, each split into its
/// fields, the header left out.
fn statistics(output: &std::process::Output) -> Vec<Vec<String>> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| line.strip_prefix("stats\t"))
        .skip(1)
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

/// The measurement memo keeps its tables in the run's derivation store
/// (#451): `--stats` lists them beside the run's kinds, each without a
/// version (`-`), and keeping one byte of derivations releases the
/// memo's values like any other kind's, so values the `spans` case's
/// rules share are measured again when read again. The result is the
/// same either way.
#[test]
fn the_stats_list_the_measurement_memo_with_the_run_kinds() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("memo-stats");
    std::fs::create_dir_all(&directory).unwrap();
    let model = directory.join("building.ifc");
    std::fs::write(&model, building(1)).unwrap();
    let check = Check {
        case: "spans",
        model: ("building.ifc".to_owned(), model),
        ruleset: case_ruleset("spans"),
    };
    let kept = run(&check, &["--threads", "1", "--stats"]);
    let released = run(
        &check,
        &["--threads", "1", "--stats", "--keep-derivations", "1"],
    );
    assert_eq!(kept.stdout, released.stdout);
    // The memo's tables, by their hits, builds and evictions together.
    let memo = |output: &std::process::Output| {
        let tables: Vec<Vec<String>> = statistics(output)
            .into_iter()
            .filter(|line| line[1] == "-")
            .collect();
        assert!(
            !tables.is_empty(),
            "no memo table: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let sum = |field: usize| -> u64 {
            tables
                .iter()
                .map(|line| line[field].parse::<u64>().expect("a count"))
                .sum()
        };
        (sum(2), sum(3), sum(8))
    };
    let (hits, builds, evicted) = memo(&kept);
    assert!(hits > 0 && evicted == 0, "{hits} hits, {evicted} evicted");
    let (_, rebuilt, released) = memo(&released);
    assert!(
        released > 0 && rebuilt > builds,
        "{released} released, {rebuilt} builds against {builds}"
    );
}

/// [`differences`] under the budget of [`BUDGETED`].
fn differences_budgeted(checks: &[Check]) -> (Vec<String>, Vec<Vec<u8>>) {
    differences(checks, BUDGETED)
}

/// Every variant reports what the first does over each pinned public
/// model (`fixtures/parity/models.json`), each checked against its digest.
#[test]
#[ignore = "needs the public models in AXIOVAL_PARITY_MODELS (scripts/parity_models.py fetch)"]
fn public_models_are_judged_alike_whatever_the_order_threads_and_reuse() {
    use sha2::{Digest, Sha256};
    let directory = PathBuf::from(
        std::env::var_os("AXIOVAL_PARITY_MODELS")
            .expect("set AXIOVAL_PARITY_MODELS to the fetched public models"),
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(FIXTURES).join("models.json")).unwrap(),
    )
    .unwrap();
    let models: Vec<(String, PathBuf)> = manifest["models"]
        .as_array()
        .expect("`models` lists the pinned models")
        .iter()
        .map(|model| {
            let name = model["name"].as_str().expect("a model name").to_owned();
            let path = directory.join(&name);
            let bytes = std::fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "{}: {error}; fetch it with scripts/parity_models.py fetch",
                    path.display()
                )
            });
            let digest = Sha256::digest(bytes)
                .iter()
                .fold(String::new(), |mut hex, byte| {
                    let _ = write!(hex, "{byte:02x}");
                    hex
                });
            assert_eq!(
                digest,
                model["sha256"].as_str().expect("a pinned digest"),
                "{}: not the pinned model; fetch it again",
                path.display()
            );
            (name, path)
        })
        .collect();
    let checks: Vec<Check> = CASES
        .iter()
        .flat_map(|case| {
            models.iter().map(|model| Check {
                case,
                model: model.clone(),
                ruleset: case_ruleset(case),
            })
        })
        .collect();
    let (mut differences, _) = differences(&checks, VARIANTS);
    differences.extend(differences_budgeted(&checks).0);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
