//! ifcZIP: the one model of an archive is read as the plain file is, and
//! archives that do not state one model safely are refused.
#![allow(missing_docs)]

use std::io::{Cursor, Write};

use axioval_ifc::{
    IfcZipError, import_ifc_session, import_ifc_zip_session, is_ifc_zip, read_ifc_zip,
};
use axioval_ir::SourceId;
use zip::write::SimpleFileOptions;

const MODEL: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n#1=IFCWALL('0000000000000000000001',$,'Wall',$,$,$,$,$,$);\n#2=IFCDOOR('0000000000000000000002',$,'Door',$,$,$,$,$,$,$,$,$,$);\nENDSEC;\nEND-ISO-10303-21;\n";

/// An archive of `members` (path, content), deflated.
fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in members {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn the_one_model_of_an_archive_reads_as_the_plain_file() {
    let zipped = archive(&[
        ("readme.txt", b"not a model"),
        ("models/house.IFC", MODEL.as_bytes()),
    ]);
    assert!(is_ifc_zip(&zipped));
    assert!(!is_ifc_zip(MODEL.as_bytes()));
    let member = read_ifc_zip(&zipped).unwrap();
    assert_eq!(member.name(), "models/house.IFC");
    assert_eq!(member.bytes(), MODEL.as_bytes());
    assert_eq!(
        member.document("house.ifczip"),
        "house.ifczip/models/house.IFC"
    );

    let plain = import_ifc_session("house.ifc", MODEL.as_bytes()).unwrap();
    let zipped = import_ifc_zip_session("house.ifczip", &zipped).unwrap();
    let snapshot = zipped.snapshots().next().unwrap();
    assert_eq!(
        snapshot.source(),
        &SourceId::new("ifc-step", "house.ifczip/models/house.IFC").unwrap()
    );
    // The same bytes: the same fingerprint and the same objects.
    assert_eq!(
        snapshot.fingerprint(),
        plain.snapshots().next().unwrap().fingerprint()
    );
    let objects = |session: &axioval_engine::EvidenceSession| -> Vec<(String, String)> {
        session
            .project()
            .objects()
            .map(|object| (object.id.local_id.clone(), object.kind().to_owned()))
            .collect()
    };
    assert_eq!(objects(&plain), objects(&zipped));
    assert_eq!(objects(&zipped).len(), 2);
}

#[test]
fn an_archive_with_two_models_is_refused() {
    let zipped = archive(&[("a.ifc", MODEL.as_bytes()), ("b.ifc", MODEL.as_bytes())]);
    let error = read_ifc_zip(&zipped).unwrap_err();
    assert_eq!(
        error,
        IfcZipError::SeveralModels(vec!["a.ifc".to_owned(), "b.ifc".to_owned()])
    );
    assert_eq!(
        error.to_string(),
        "the ifcZIP archive holds 2 models (a.ifc, b.ifc); exactly one is read"
    );
    // An IFC-XML beside a STEP model is a second model too.
    let zipped = archive(&[("a.ifc", MODEL.as_bytes()), ("a.ifcXML", b"<ifc/>")]);
    assert!(matches!(
        read_ifc_zip(&zipped),
        Err(IfcZipError::SeveralModels(_))
    ));
}

#[test]
fn an_archive_without_a_readable_model_is_refused() {
    let zipped = archive(&[("readme.txt", b"nothing")]);
    assert_eq!(read_ifc_zip(&zipped), Err(IfcZipError::NoModel));
    let zipped = archive(&[("m.ifcxml", b"<ifc/>")]);
    assert_eq!(
        read_ifc_zip(&zipped),
        Err(IfcZipError::UnsupportedModel("m.ifcxml".to_owned()))
    );
    assert!(matches!(
        read_ifc_zip(MODEL.as_bytes()),
        Err(IfcZipError::Archive(_))
    ));
    // A member the session refuses names the member.
    let zipped = archive(&[("m.ifc", b"not STEP")]);
    let Err(error) = import_ifc_zip_session("m.ifczip", &zipped) else {
        panic!("a member that is not STEP was read");
    };
    assert!(
        matches!(&error, IfcZipError::Session { member, .. } if member == "m.ifc"),
        "{error}"
    );
}

#[test]
fn unsafe_member_paths_refuse_the_archive() {
    for name in ["../m.ifc", "/abs/m.ifc", "dir/../../m.ifc", "dir\\m.ifc"] {
        let zipped = archive(&[("ok.ifc", MODEL.as_bytes()), (name, b"x")]);
        assert_eq!(
            read_ifc_zip(&zipped),
            Err(IfcZipError::UnsafePath(name.to_owned())),
            "{name}"
        );
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("m.ifc", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    let zipped = writer.finish().unwrap().into_inner();
    assert_eq!(
        read_ifc_zip(&zipped),
        Err(IfcZipError::UnsafePath("m.ifc".to_owned()))
    );
}
