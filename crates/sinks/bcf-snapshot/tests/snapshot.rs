//! Snapshots of a clash topic's viewpoints, rendered through the BCF sink
//! and read back from the archive.
#![allow(missing_docs, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use axioval_bcf::{
    Bounds, IFC_GLOBAL_ID_SCHEME, Options, SNAPSHOT_NOTE, SnapshotRenderer, SnapshotView, Version,
    export, export_with_snapshots,
};
use axioval_bcf_snapshot::{CONTEXT_COLOR, Mesh, Renderer};
use axioval_ir::{
    Evidence, ExternalId, Finding, Object, ObjectId, Project, Report, RuleId, Severity, SourceId,
};

fn id(local: u64) -> ObjectId {
    ObjectId::new(
        SourceId::new("ifc-step", "a.ifc").unwrap(),
        format!("#{local}"),
    )
    .unwrap()
}

/// A wall and a duct that clash, and a column beside them.
fn model() -> Project {
    let alias = |value: &str| ExternalId::new(IFC_GLOBAL_ID_SCHEME, value).unwrap();
    Project::new(vec![
        Object::new(id(1), "IFCWALL").with_external_id(alias("0000000000000000000001")),
        Object::new(id(2), "IFCDUCTSEGMENT").with_external_id(alias("0000000000000000000002")),
        Object::new(id(3), "IFCCOLUMN").with_external_id(alias("0000000000000000000003")),
    ])
    .unwrap()
}

fn report() -> Report {
    let source = SourceId::new("ifc-step", "a.ifc").unwrap();
    let clash = Finding::new(
        RuleId::new("clash").unwrap(),
        id(1),
        Severity::Error,
        "Wall clashes with duct",
    )
    .with_related([id(2)])
    .with_evidence([Evidence::exact(source, "clash:1")]);
    Report {
        findings: vec![clash],
        ..Report::default()
    }
}

/// The corners of each object's box: the wall, the duct through it, and
/// the column a metre and a half away.
fn boxes() -> [(ObjectId, [f64; 3], [f64; 3]); 3] {
    [
        (id(1), [0.0, 0.0, 0.0], [3.0, 0.3, 3.0]),
        (id(2), [1.0, -1.0, 1.0], [1.6, 1.3, 1.6]),
        (id(3), [-2.5, 0.0, 0.0], [-2.0, 0.5, 3.0]),
    ]
}

/// A closed box as twelve triangles.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> Mesh {
    let corner = |i: usize| {
        [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ]
    };
    let positions = (0..8).map(corner).collect();
    let faces = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    let triangles = faces
        .iter()
        .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
        .collect();
    Mesh::new(positions, triangles).unwrap()
}

fn renderer() -> Renderer {
    Renderer::new(
        boxes()
            .into_iter()
            .map(|(id, min, max)| (id, cuboid(min, max)))
            .collect(),
    )
    .with_height(128)
}

fn options(version: Version) -> Options {
    Options {
        version,
        bounds: Some(
            boxes()
                .into_iter()
                .map(|(id, min, max)| (id, Bounds::new(min, max).unwrap()))
                .collect(),
        ),
        ..Options::new("axioval", "2026-09-30T10:00:00Z")
    }
}

/// Every PNG in the archive, by entry name.
fn pngs(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut found = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if std::path::Path::new(entry.name())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            found.insert(entry.name().to_owned(), data);
        }
    }
    found
}

/// The pixels of a PNG this crate wrote: 8-bit RGB, rows unfiltered.
fn decode(png: &[u8]) -> (u32, u32, Vec<[u8; 3]>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (mut at, mut width, mut height, mut data) = (8, 0, 0, Vec::new());
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = &png[at + 4..at + 8];
        let body = &png[at + 8..at + 8 + length];
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes(body[..4].try_into().unwrap());
                height = u32::from_be_bytes(body[4..8].try_into().unwrap());
                assert_eq!(&body[8..], [8, 2, 0, 0, 0]);
            }
            b"IDAT" => data.extend_from_slice(body),
            _ => {}
        }
        at += 12 + length;
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&data[..])
        .read_to_end(&mut raw)
        .unwrap();
    let mut pixels = Vec::new();
    for row in raw.chunks_exact(width as usize * 3 + 1) {
        assert_eq!(row[0], 0, "unfiltered rows");
        pixels.extend(row[1..].chunks_exact(3).map(|p| [p[0], p[1], p[2]]));
    }
    assert_eq!(pixels.len(), (width * height) as usize);
    (width, height, pixels)
}

fn red(p: [u8; 3]) -> bool {
    p[0] > 0 && p[1] == 0 && p[2] == 0
}

fn blue(p: [u8; 3]) -> bool {
    p[2] > 0 && p[0] == 0 && p[1] == 0
}

fn grey(p: [u8; 3]) -> bool {
    p[0] == p[1] && p[1] == p[2] && p[0] < CONTEXT_COLOR[0].saturating_add(1) && p[0] > 0x40
}

#[test]
fn a_clash_snapshot_shows_both_elements_highlighted_from_the_viewpoint_camera() {
    for version in [Version::V2_1, Version::V3_0] {
        let export =
            export_with_snapshots(&report(), &model(), &options(version), &renderer()).unwrap();
        assert!(export.unrendered.is_empty());
        let topic = &export.document.topics[0];
        assert_eq!(topic.viewpoints.len(), 2);
        assert!(
            topic
                .description
                .as_deref()
                .unwrap()
                .ends_with(SNAPSHOT_NOTE)
        );
        let bytes = export.to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let markup = archive.topics().next().unwrap();
        let images = pngs(&bytes);
        assert_eq!(images.len(), 2, "one per viewpoint");
        for reference in &markup.viewpoints {
            let name = reference.snapshot.as_deref().unwrap();
            let entry = images
                .iter()
                .find(|(entry, _)| entry.ends_with(name))
                .unwrap();
            let (width, height, pixels) = decode(entry.1);
            assert_eq!((width, height), (128, 128));
            let reds = pixels.iter().filter(|p| red(**p)).count();
            let blues = pixels.iter().filter(|p| blue(**p)).count();
            assert!(
                reds > 200 && blues > 50,
                "{version:?}: {reds} red, {blues} blue"
            );
            // The camera frames the two: the image centre shows one of them.
            let centre = pixels[(height / 2 * width + width / 2) as usize];
            assert!(red(centre) || blue(centre), "{centre:?}");
        }
    }
}

/// Renders the perspective viewpoint of the clash with `options`.
fn perspective_pixels(options: &Options) -> Vec<[u8; 3]> {
    let export = export_with_snapshots(&report(), &model(), options, &renderer()).unwrap();
    let png = &export.document.topics[0].viewpoints[0]
        .snapshot
        .as_ref()
        .unwrap()
        .png;
    decode(png).2
}

#[test]
fn context_is_grey_unless_isolated_or_cut_away() {
    let base = options(Version::V2_1);
    assert!(perspective_pixels(&base).iter().any(|p| grey(*p)));
    let isolated = Options {
        isolate: true,
        ..base.clone()
    };
    assert!(!perspective_pixels(&isolated).iter().any(|p| grey(*p)));
    // The section box around wall and duct cuts the column away.
    let boxed = Options {
        section_box: true,
        ..base
    };
    let pixels = perspective_pixels(&boxed);
    assert!(!pixels.iter().any(|p| grey(*p)));
    assert!(pixels.iter().any(|p| red(*p)) && pixels.iter().any(|p| blue(*p)));
}

#[test]
fn configured_colours_are_drawn() {
    let options = Options {
        colors: Some(axioval_bcf::Colors {
            subject: "00FF00".parse().unwrap(),
            related: "FF00FF".parse().unwrap(),
        }),
        ..options(Version::V2_1)
    };
    let pixels = perspective_pixels(&options);
    assert!(pixels.iter().any(|p| p[1] > 0 && p[0] == 0 && p[2] == 0));
    assert!(pixels.iter().any(|p| p[0] > 0 && p[0] == p[2] && p[1] == 0));
}

#[test]
fn identical_input_renders_identical_bytes() {
    let bytes = || {
        export_with_snapshots(&report(), &model(), &options(Version::V2_1), &renderer())
            .unwrap()
            .to_bytes()
            .unwrap()
    };
    assert_eq!(bytes(), bytes());
}

/// A renderer that draws nothing.
struct Declining;

impl SnapshotRenderer for Declining {
    fn render(&self, _: &SnapshotView<'_>) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn without_snapshots_the_archive_is_byte_identical() {
    let options = options(Version::V2_1);
    let plain = export(&report(), &model(), &options).unwrap();
    let declined = export_with_snapshots(&report(), &model(), &options, &Declining).unwrap();
    assert_eq!(
        plain.to_bytes().unwrap(),
        declined.to_bytes().unwrap(),
        "a declined snapshot writes nothing"
    );
    assert!(plain.unrendered.is_empty());
    assert_eq!(declined.unrendered, [id(1)]);
    assert!(pngs(&plain.to_bytes().unwrap()).is_empty());

    // A subject without a mesh is declined by the renderer too.
    let meshless = Renderer::new(BTreeMap::from([(id(2), cuboid([0.0; 3], [1.0; 3]))]));
    let export = export_with_snapshots(&report(), &model(), &options, &meshless).unwrap();
    assert_eq!(export.to_bytes().unwrap(), plain.to_bytes().unwrap());
    assert_eq!(export.unrendered, [id(1)]);
}

#[test]
fn a_viewpoint_without_a_camera_gets_no_snapshot() {
    let options = Options::new("axioval", "2026-09-30T10:00:00Z");
    let plain = export(&report(), &model(), &options).unwrap();
    let rendered = export_with_snapshots(&report(), &model(), &options, &renderer()).unwrap();
    assert_eq!(plain.to_bytes().unwrap(), rendered.to_bytes().unwrap());
    assert!(rendered.unrendered.is_empty());
}

#[test]
fn meshes_with_bad_coordinates_or_indices_are_refused() {
    assert!(Mesh::new(vec![[0.0, f64::NAN, 0.0]], vec![]).is_none());
    assert!(Mesh::new(vec![[0.0; 3]], vec![[0, 0, 1]]).is_none());
}
