//! Which object definitions a session checks.
//!
//! Occurrences and, in IFC4, contexts such as `IfcProject` are session
//! objects: both answer their properties exactly. A type object is not yet
//! one, although its own property sets now resolve; a resource is not either.
#![allow(missing_docs)]

use axioval_engine::{
    EvidenceSession, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, SourceId};

const MODEL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('0000000000000000000001',$,'P',$,$,$,$,$,$);
#2=IFCPROPERTYSINGLEVALUE('Phase',$,IFCLABEL('Design'),$);
#3=IFCPROPERTYSET('0000000000000000000003',$,'Pset_ProjectCommon',$,(#2));
#4=IFCRELDEFINESBYPROPERTIES('0000000000000000000004',$,$,$,(#1),#3);
#5=IFCPROJECTLIBRARY('0000000000000000000005',$,'L',$,$,$,$,$,$);
#10=IFCWALLTYPE('0000000000000000000010',$,'T',$,$,(#13),$,$,'Custom',.USERDEFINED.);
#11=IFCWALL('0000000000000000000011',$,$,$,$,$,$,$,$);
#12=IFCRELDEFINESBYTYPE('0000000000000000000012',$,$,$,(#11),#10);
#13=IFCPROPERTYSET('0000000000000000000013',$,'Pset_WallCommon',$,(#14));
#14=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 60'),$);
#30=IFCMATERIAL('Concrete',$,$);
#31=IFCRELASSOCIATESMATERIAL('0000000000000000000031',$,$,$,(#11),#30);
ENDSEC;
END-ISO-10303-21;
";

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

fn session() -> EvidenceSession {
    import_ifc_session("model.ifc", MODEL.as_bytes()).unwrap()
}

fn resolve(
    session: &EvidenceSession,
    local: &str,
    set: &str,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&PropertyRequest::try_new(id(local), Some(set.into()), name).unwrap())
}

fn value(resolution: Result<PropertyResolution, PropertyResolutionError>) -> PropertyValue {
    match resolution {
        Ok(PropertyResolution::Present(resolved)) => resolved.property().value().clone(),
        other => panic!("expected a value, got {other:?}"),
    }
}

#[test]
fn occurrences_and_contexts_are_objects_and_types_and_resources_are_not() {
    let session = session();
    let mut kinds: Vec<(String, String)> = session
        .project()
        .objects()
        .map(|object| (object.id.local_id.clone(), object.kind().to_owned()))
        .collect();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            ("#1".to_owned(), "IFCPROJECT".to_owned()),
            ("#11".to_owned(), "IFCWALL".to_owned()),
            ("#5".to_owned(), "IFCPROJECTLIBRARY".to_owned()),
        ]
    );
    // A context keeps its source-qualified identity and GlobalId alias.
    let project = session.project().object(&id("#1")).unwrap();
    assert_eq!(
        project.external_id("ifc-globalid"),
        Some("0000000000000000000001")
    );
}

#[test]
fn an_ifc4_project_answers_its_properties_and_attributes() {
    let session = session();
    assert_eq!(
        value(resolve(&session, "#1", "Pset_ProjectCommon", "Phase")),
        PropertyValue::String("Design".into())
    );
    assert_eq!(
        value(resolve(&session, "#1", ATTRIBUTE_SET, "Name")),
        PropertyValue::String("P".into())
    );
    assert!(matches!(
        resolve(&session, "#1", "Pset_ProjectCommon", "Missing"),
        Ok(PropertyResolution::Absent(_))
    ));
}

#[test]
fn a_type_objects_own_property_sets_resolve_with_type_provenance() {
    // `ifc-properties` 0.5.1 resolves a type object's own `HasPropertySets`,
    // with the provenance an occurrence of that type reports for the set.
    let session = session();
    let own = locator(resolve(&session, "#10", "Pset_WallCommon", "FireRating"));
    assert!(own.ends_with(":type:#10:#13/#14"), "{own}");
    assert_eq!(
        value(resolve(&session, "#10", "Pset_WallCommon", "FireRating")),
        PropertyValue::String("EI 60".into())
    );
    assert!(matches!(
        resolve(&session, "#10", "Pset_WallCommon", "Missing"),
        Ok(PropertyResolution::Absent(_))
    ));
    // The occurrence inherits the same set, with the same provenance.
    assert_eq!(
        locator(resolve(&session, "#11", "Pset_WallCommon", "FireRating")),
        own
    );
}

fn locator(resolution: Result<PropertyResolution, PropertyResolutionError>) -> String {
    match resolution {
        Ok(PropertyResolution::Present(resolved)) => {
            resolved
                .property()
                .evidence
                .clone()
                .expect("exact evidence")
                .locator
        }
        other => panic!("expected a value, got {other:?}"),
    }
}
