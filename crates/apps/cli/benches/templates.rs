//! Each built-in capability rebuilt as a template, timed and weighed
//! against the implementation it replaced (#279).
//!
//! For every pair in [`PAIRS`] and every input (a generated fixture model
//! and each pinned public model the pair's parity case names), the model is
//! imported and meshed once, as `axioval check --geometry` does, and the
//! case's rules of the capability are run by two runtimes over the same
//! session: one with the default registry (the template) and one with the
//! replaced implementation registered in its place (the reference). Both
//! must agree under the parity contract; then they run interleaved, after a
//! warm-up, and each run's wall time and peak heap (a counting global
//! allocator, over what the run allocates beyond what was live before it)
//! is recorded.
//!
//! The bench prints a line per input and, with `--out FILE`, writes one
//! JSON record per input; `scripts/bench.py` runs it and judges the records
//! against the recorded budget (`scripts/bench_budget.json`):
//!
//! ```text
//! python3 scripts/bench.py gate      # fails when a template exceeds its budget
//! python3 scripts/bench.py report    # prints the same, never fails on a ratio
//! ```
//!
//! Environment: `AXIOVAL_PARITY_MODELS` (the fetched public models,
//! default `~/.cache/axioval/parity-models`), `AXIOVAL_BENCH_RUNS`
//! (measured runs per side, default 21), `AXIOVAL_BENCH_WALLS` (walls in the
//! generated fixture, default 400) and `AXIOVAL_BENCH_RULES` (rule ids,
//! comma-separated, to run alone while profiling).
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axioval::engine::{CapabilityRegistry, EvidenceSession, ExecutionPlan, Runtime};
use axioval::ir::{DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::parity::{Observations, Parity};
use peak_alloc::PeakAlloc;
use serde_json::{Value, json};

#[global_allocator]
static HEAP: PeakAlloc = PeakAlloc;

// The IFC to Axiolid bridge of the binary, meshing exactly as `check
// --geometry` does. The bench uses only `attach`.
#[allow(dead_code, unused_imports, clippy::all, clippy::pedantic)]
#[path = "../src"]
mod cli {
    pub mod geometry;
}
use cli::geometry;

/// A capability rebuilt as a template, and the implementation it replaced.
struct Pair {
    /// The capability's id.
    capability: &'static str,
    /// The parity case (`fixtures/parity/cases/<case>`) whose rules of the
    /// capability are run.
    case: &'static str,
    /// The registry with the replaced implementation in the template's
    /// place.
    reference: fn(CapabilityRegistry) -> Result<CapabilityRegistry, axioval::engine::EngineError>,
}

/// Every rebuilt capability with a live reference. A rebuild adds its
/// pair here when it moves the implementation behind `parity-reference`.
const PAIRS: &[Pair] = &[
    Pair {
        capability: "axioval:capability.body-extent",
        case: "elements",
        reference: |registry| registry.replace(axioval::rules::reference::BodyExtent),
    },
    Pair {
        capability: "axioval:capability.property-predicate",
        case: "elements",
        reference: |registry| registry.replace(axioval::rules::reference::PropertyPredicate),
    },
    Pair {
        capability: "axioval:capability.triangle-count",
        case: "elements",
        reference: |registry| registry.replace(axioval::rules::reference::TriangleCountLimit),
    },
    Pair {
        capability: "axioval:capability.plan-area",
        case: "storeys",
        reference: |registry| registry.replace(axioval::rules::reference::PlanAreaRange),
    },
    Pair {
        capability: "axioval:capability.level-spacing",
        case: "storeys",
        reference: |registry| registry.replace(axioval::rules::reference::LevelSpacing),
    },
    Pair {
        capability: "axioval:capability.object-count",
        case: "counts",
        reference: |registry| registry.replace(axioval::rules::reference::ObjectCount),
    },
    Pair {
        capability: "axioval:capability.related-count",
        case: "counts",
        reference: |registry| registry.replace(axioval::rules::reference::RelatedCount),
    },
    Pair {
        capability: "axioval:capability.unique-value",
        case: "counts",
        reference: |registry| registry.replace(axioval::rules::reference::UniqueValue),
    },
    Pair {
        capability: "axioval:capability.shelf-capacity",
        case: "shelving",
        reference: |registry| registry.replace(axioval::rules::reference::ShelfCapacity),
    },
    Pair {
        capability: "axioval:capability.area-ratio",
        case: "storeys",
        reference: |registry| registry.replace(axioval::rules::reference::AreaRatio),
    },
    Pair {
        capability: "axioval:capability.plan-coverage",
        case: "storeys",
        reference: |registry| registry.replace(axioval::rules::reference::PlanCoverage),
    },
    Pair {
        capability: "axioval:capability.consistent-value",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::ConsistentValue),
    },
    Pair {
        capability: "axioval:capability.selector-conformance",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::SelectorConformance),
    },
    Pair {
        capability: "axioval:capability.relative-count",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::RelativeCount),
    },
    Pair {
        capability: "axioval:capability.property-value",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::PropertyValueConstraint),
    },
    Pair {
        capability: "axioval:capability.property-requirements",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::PropertyRequirements),
    },
    Pair {
        capability: "axioval:capability.property-comparison",
        case: "judges",
        reference: |registry| registry.replace(axioval::rules::reference::PropertyComparison),
    },
    Pair {
        capability: "axioval:capability.ramp-geometry",
        case: "stairs",
        reference: |registry| registry.replace(axioval::rules::reference::RampGeometry),
    },
    Pair {
        capability: "axioval:capability.stair-geometry",
        case: "stairs",
        reference: |registry| registry.replace(axioval::rules::reference::StairGeometry),
    },
];

/// Warm-up runs per side before measuring.
const WARM_UP: usize = 3;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?)
}

/// A pair's parity case, cut down to the capability's rules.
struct Case {
    definitions: DefinitionPackage,
    ruleset: RuleSetPackage,
    /// The rules kept, by id.
    rules: Vec<String>,
    /// The public models the case names.
    models: Vec<String>,
}

impl Case {
    /// The case's packages, its ruleset cut down to the rules whose
    /// definition is bound to the pair's capability.
    /// `None` when `AXIOVAL_BENCH_RULES` names none of them.
    fn of(pair: &Pair) -> Result<Option<Self>, Box<dyn Error>> {
        let case = root().join("fixtures/parity/cases").join(pair.case);
        let definitions = read_json(&case.join("definitions.json"))?;
        let bound: Vec<&str> = definitions["definitions"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, definition)| definition["capability"] == pair.capability)
            .map(|(id, _)| id.as_str())
            .collect();
        let only = std::env::var("AXIOVAL_BENCH_RULES")
            .ok()
            .filter(|only| !only.is_empty());
        let mut ruleset = read_json(&case.join("ruleset.json"))?;
        let mut rules = Vec::new();
        retain(&mut ruleset["root"], (&bound, only.as_deref()), &mut rules);
        if rules.is_empty() && only.is_some() {
            return Ok(None);
        }
        if rules.is_empty() {
            return Err(format!("case `{}` has no rule of {}", pair.case, pair.capability).into());
        }
        let parity = read_json(&case.join("parity.json"))?;
        let models = match &parity["models"] {
            Value::Array(names) => names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => public_models()?,
        };
        Ok(Some(Self {
            definitions: serde_json::from_value(definitions)?,
            ruleset: serde_json::from_value(ruleset)?,
            rules,
            models,
        }))
    }
}

/// Keeps the rules of `folder` (and its folders) bound to `bound`, and of
/// those only the ones `only` names, if it names any. The cases' rules read
/// no other rule's outcome, so none is needed beyond them.
fn retain(folder: &mut Value, (bound, only): (&[&str], Option<&str>), kept: &mut Vec<String>) {
    if let Some(rules) = folder.get_mut("rules").and_then(Value::as_array_mut) {
        rules.retain(|rule| {
            let id = rule["id"].as_str().unwrap_or_default();
            let keep = rule["definitionId"]
                .as_str()
                .is_some_and(|definition| bound.contains(&definition))
                && only.is_none_or(|only| only.split(',').any(|named| named == id));
            if keep {
                kept.push(id.to_owned());
            }
            keep
        });
    }
    if let Some(folders) = folder.get_mut("folders").and_then(Value::as_array_mut) {
        for child in folders {
            retain(child, (bound, only), kept);
        }
    }
}

/// Every pinned public model's name.
fn public_models() -> Result<Vec<String>, Box<dyn Error>> {
    let manifest = read_json(&root().join("fixtures/parity/models.json"))?;
    Ok(manifest["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["name"].as_str().map(str::to_owned))
        .collect())
}

fn models_dir() -> Option<PathBuf> {
    std::env::var_os("AXIOVAL_PARITY_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".cache/axioval/parity-models"))
        })
}

/// `walls` straight walls on a grid, each an extruded rectangle on its own
/// placement turned by one of a few headings, with a material layer set
/// stating its thickness, and a slab per twenty walls. Most walls pass the
/// case's rules; a share is too thick, too long, or states another
/// thickness than its body has.
fn fixture(walls: usize) -> String {
    let mut data = String::from(
        "#1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'Bench',$,$,$,$,(#5),#7);\n\
         #9=IFCMATERIAL('Concrete',$,$);\n\
         #11=IFCCARTESIANPOINT((0.,0.));\n\
         #12=IFCAXIS2PLACEMENT2D(#11,$);\n",
    );
    let headings = [0.0_f64, 30.0, 45.0, 90.0, 120.0, 210.0];
    let mut next = 100_usize;
    for index in 0..walls {
        let [
            point,
            axis,
            placement3,
            local,
            profile,
            solid,
            shape,
            product,
            wall,
            layer,
            set,
            usage,
            rel,
        ] = std::array::from_fn(|offset| next + offset);
        let base = next;
        next += 20;
        #[allow(clippy::cast_precision_loss)]
        let (x, y) = ((index % 40) as f64 * 10.0, (index / 40) as f64 * 10.0);
        let heading = headings[index % headings.len()].to_radians();
        let thickness = if index % 11 == 3 { 0.3 } else { 0.2 };
        let stated = if index % 13 == 5 { 0.25 } else { thickness };
        #[allow(clippy::cast_precision_loss)]
        let length = 3.0 + (index % 5) as f64 * 1.25;
        let guid = move |offset: usize| format!("{:022}", base * 100 + offset);
        let _ = write!(
            data,
            "#{point}=IFCCARTESIANPOINT(({x:?},{y:?},0.));\n\
             #{axis}=IFCDIRECTION(({:?},{:?},0.));\n\
             #{placement3}=IFCAXIS2PLACEMENT3D(#{point},#4,#{axis});\n\
             #{local}=IFCLOCALPLACEMENT($,#{placement3});\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#12,{length:?},{thickness:?});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{wall}=IFCWALL('{}',$,'W{index}',$,$,#{local},#{product},$,$);\n\
             #{layer}=IFCMATERIALLAYER(#9,{stated:?},$,$,$,$,$);\n\
             #{set}=IFCMATERIALLAYERSET((#{layer}),'L{index}',$);\n\
             #{usage}=IFCMATERIALLAYERSETUSAGE(#{set},.AXIS2.,.POSITIVE.,0.,$);\n\
             #{rel}=IFCRELASSOCIATESMATERIAL('{}',$,$,$,(#{wall}),#{usage});\n",
            heading.cos(),
            heading.sin(),
            guid(1),
            guid(2),
        );
        if index % 20 == 0 {
            let [profile, solid, shape, product, slab] =
                std::array::from_fn(|offset| next + offset);
            next += 10;
            let depth = if index % 40 == 0 { 0.18 } else { 0.24 };
            let _ = write!(
                data,
                "#{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#12,6.,4.);\n\
                 #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:?});\n\
                 #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
                 #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
                 #{slab}=IFCSLAB('{}',$,'S{index}',$,$,#{local},#{product},$,$);\n",
                guid(3),
            );
        }
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('bench.ifc','2026-01-01T00:00:00',(''),(''),'p','o','a');\n\
         FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// One input, imported and meshed.
struct Input {
    name: String,
    session: EvidenceSession,
}

fn input(name: &str, bytes: Vec<u8>) -> Result<Input, Box<dyn Error>> {
    let session = axioval::ifc::import_ifc_session(name, &bytes)
        .map_err(|error| format!("{name}: {error}"))?;
    let source = session
        .snapshots()
        .next()
        .map(|snapshot| snapshot.source().clone())
        .ok_or_else(|| format!("{name}: the import produced no source"))?;
    let models: geometry::ModelBytes = BTreeMap::from([(source, bytes)]);
    let (session, _) = geometry::attach(session, &models, geometry::Options::meshes(false))
        .map_err(|error| format!("{name}: geometry: {error}"))?;
    Ok(Input {
        name: name.to_owned(),
        session,
    })
}

/// One side's measured runs.
#[derive(Default)]
struct Side {
    times: Vec<Duration>,
    peaks: Vec<usize>,
}

impl Side {
    fn measure(&mut self, runtime: &Runtime, input: &Input, plan: &ExecutionPlan) -> Report {
        let plan = plan.clone();
        HEAP.reset_peak_usage();
        let base = HEAP.current_usage();
        let start = Instant::now();
        let report = runtime
            .run_session(&input.session, plan)
            .expect("the run fails");
        let elapsed = start.elapsed();
        self.peaks.push(HEAP.peak_usage().saturating_sub(base));
        self.times.push(elapsed);
        report
    }

    fn summary(&self) -> Value {
        let mut times: Vec<u128> = self.times.iter().map(Duration::as_nanos).collect();
        times.sort_unstable();
        let mut peaks = self.peaks.clone();
        peaks.sort_unstable();
        json!({
            "runs": times.len(),
            "median_ns": times[times.len() / 2],
            "min_ns": times[0],
            "max_ns": times[times.len() - 1],
            "peak_bytes": peaks[peaks.len() / 2],
        })
    }
}

fn bench(
    pair: &Pair,
    input: &Input,
    (plan, rules): (&ExecutionPlan, &[String]),
    (template, reference): (&Runtime, &Runtime),
    runs: usize,
) -> Value {
    let mut sides = [Side::default(), Side::default()];
    // Both sides must report the same before they are compared.
    let template_report = sides[0].measure(template, input, plan);
    let reference_report = sides[1].measure(reference, input, plan);
    // Under the parity contract, as the parity harness holds a template
    // to its reference: evidence locators may differ, nothing else.
    let differences: Vec<String> = rules
        .iter()
        .map(|rule| {
            Parity::contract().compare(
                (
                    "reference",
                    &Observations::of_report(&reference_report, rule),
                ),
                ("template", &Observations::of_report(&template_report, rule)),
            )
        })
        .filter(|evidence| !evidence.holds())
        .map(|evidence| evidence.diff())
        .collect();
    drop((template_report, reference_report));
    for _ in 1..WARM_UP {
        sides[0].measure(template, input, plan);
        sides[1].measure(reference, input, plan);
    }
    sides = [Side::default(), Side::default()];
    // Interleaved, alternating which side runs first, so drift in the
    // machine's load falls on both.
    for run in 0..runs {
        if run % 2 == 0 {
            sides[0].measure(template, input, plan);
            sides[1].measure(reference, input, plan);
        } else {
            sides[1].measure(reference, input, plan);
            sides[0].measure(template, input, plan);
        }
    }
    let [template, reference] = sides.map(|side| side.summary());
    let ratio = |field: &str| {
        #[allow(clippy::cast_precision_loss)]
        let ratio = template[field].as_f64().unwrap_or(0.0)
            / reference[field].as_f64().unwrap_or(1.0).max(1.0);
        ratio
    };
    json!({
        "capability": pair.capability,
        "input": input.name,
        "objects": input.session.project().objects().count(),
        "parity": differences.is_empty(),
        "differences": differences,
        "template": template,
        "reference": reference,
        "time_ratio": ratio("median_ns"),
        "memory_ratio": ratio("peak_bytes"),
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|arg| arg == "--out")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from);
    let runs = setting("AXIOVAL_BENCH_RUNS", 21);
    let walls = setting("AXIOVAL_BENCH_WALLS", 400);
    let models = models_dir();
    let mut records = Vec::new();
    for pair in PAIRS {
        let Some(case) = Case::of(pair)? else {
            continue;
        };
        let templates = axioval::default_registry()?;
        let plan =
            axioval::engine::compile_rulesets(&templates, &[case.definitions], &[case.ruleset])?;
        let reference = Runtime::new((pair.reference)(axioval::default_registry()?)?);
        let template = Runtime::new(templates);
        let mut inputs = vec![input(
            &format!("fixture-{walls}-walls.ifc"),
            fixture(walls).into_bytes(),
        )?];
        let mut missing = Vec::new();
        for name in &case.models {
            match models
                .as_ref()
                .map(|dir| dir.join(name))
                .filter(|path| path.is_file())
            {
                Some(path) => inputs.push(input(name, std::fs::read(&path)?)?),
                None => missing.push(name.clone()),
            }
        }
        for input in &inputs {
            let record = bench(
                pair,
                input,
                (&plan, &case.rules),
                (&template, &reference),
                runs,
            );
            println!("{}", line(&record));
            records.push(record);
        }
        for name in missing {
            let record = json!({"capability": pair.capability, "input": name, "missing": true});
            println!("{}: {name}: not fetched, skipped", pair.capability);
            records.push(record);
        }
    }
    if let Some(out) = out {
        let mut text = String::new();
        for record in &records {
            text.push_str(&record.to_string());
            text.push('\n');
        }
        std::fs::write(&out, text).map_err(|error| format!("{}: {error}", out.display()))?;
    }
    Ok(())
}

/// One measurement as a line of the table.
fn line(record: &Value) -> String {
    let micros = |side: &str| record[side]["median_ns"].as_f64().unwrap_or(0.0) / 1000.0;
    let kib = |side: &str| record[side]["peak_bytes"].as_f64().unwrap_or(0.0) / 1024.0;
    format!(
        "{} {:<52} objects {:>5}  time {:>10.1} µs vs {:>10.1} µs ({:.2}×)  peak {:>9.1} KiB vs {:>9.1} KiB ({:.2}×){}",
        record["capability"].as_str().unwrap_or_default(),
        record["input"].as_str().unwrap_or_default(),
        record["objects"],
        micros("template"),
        micros("reference"),
        record["time_ratio"].as_f64().unwrap_or(0.0),
        kib("template"),
        kib("reference"),
        record["memory_ratio"].as_f64().unwrap_or(0.0),
        if record["parity"] == true {
            ""
        } else {
            "  PARITY FAILS"
        },
    )
}
