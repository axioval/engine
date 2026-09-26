//! Materials and the wholes an object is part of, as IDS reads them.
#![allow(missing_docs)]

use axioval_engine::{
    Decomposition, DecompositionError, DecompositionServiceHandle, EvidenceSession, MaterialError,
    MaterialServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

fn session(schema: &str, data: &str) -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

const MATERIALS: &str = "#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCMATERIAL('Concrete',$,'Load-bearing');
#3=IFCRELASSOCIATESMATERIAL('0000000000000000000003',$,$,$,(#1),#2);
#4=IFCWALL('0000000000000000000004',$,$,$,$,$,$,$,$);
#5=IFCMATERIAL('Brick',$,$);
#6=IFCMATERIALLAYER(#5,0.2,$,'Outer',$,'Masonry',$);
#7=IFCMATERIALLAYERSET((#6),'Wall build-up',$);
#8=IFCMATERIALLAYERSETUSAGE(#7,.AXIS2.,.POSITIVE.,0.,$);
#9=IFCRELASSOCIATESMATERIAL('0000000000000000000009',$,$,$,(#4),#8);
#10=IFCWALL('0000000000000000000010',$,$,$,$,$,$,$,$);
#11=IFCWALLTYPE('0000000000000000000011',$,$,$,$,$,$,$,$,.SHEAR.);
#12=IFCRELASSOCIATESMATERIAL('0000000000000000000012',$,$,$,(#11),#2);
#13=IFCRELDEFINESBYTYPE('0000000000000000000013',$,$,$,(#10),#11);
#14=IFCWALL('0000000000000000000014',$,$,$,$,$,$,$,$);
";

fn names(session: &EvidenceSession, local: &str) -> Result<Option<Vec<String>>, MaterialError> {
    session
        .service::<MaterialServiceHandle>()
        .unwrap()
        .material(&id(local))
        .map(|material| material.map(|material| material.names))
}

#[test]
fn materials_are_named_by_every_name_and_category_they_state() {
    let session = session("IFC4", MATERIALS);
    let strings = |names: &[&str]| Some(names.iter().map(|name| (*name).to_owned()).collect());
    assert_eq!(
        names(&session, "#1"),
        Ok(strings(&["Concrete", "Load-bearing"]))
    );
    // A usage stands for its layer set: set, layer and material names.
    assert_eq!(
        names(&session, "#4"),
        Ok(strings(&["Brick", "Masonry", "Outer", "Wall build-up"]))
    );
    // Inherited from the type.
    assert_eq!(
        names(&session, "#10"),
        Ok(strings(&["Concrete", "Load-bearing"]))
    );
    assert_eq!(names(&session, "#14"), Ok(None));
}

#[test]
fn ifc2x3_materials_are_refused_not_read_with_ifc4_slots() {
    let session = session(
        "IFC2X3",
        "#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$);\n#2=IFCMATERIAL('Concrete');\n#3=IFCRELASSOCIATESMATERIAL('0000000000000000000003',$,$,$,(#1),#2);\n",
    );
    assert!(matches!(
        names(&session, "#1"),
        Err(MaterialError::Unsupported(_))
    ));
}

const STRUCTURE: &str = "#20=IFCBUILDING('0000000000000000000020',$,$,$,$,$,$,$,$,$,$,$);
#21=IFCBUILDINGSTOREY('0000000000000000000021',$,$,$,$,$,$,$,$,$);
#22=IFCRELAGGREGATES('0000000000000000000022',$,$,$,#20,(#21));
#23=IFCSPACE('0000000000000000000023',$,$,$,$,$,$,$,$,.INTERNAL.,$);
#24=IFCRELAGGREGATES('0000000000000000000024',$,$,$,#21,(#23));
#25=IFCBEAM('0000000000000000000025',$,$,$,$,$,$,$,$);
#26=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000026',$,$,$,(#25),#23);
#27=IFCWALL('0000000000000000000027',$,$,$,$,$,$,$,$);
#28=IFCOPENINGELEMENT('0000000000000000000028',$,$,$,$,$,$,$,$);
#29=IFCRELVOIDSELEMENT('0000000000000000000029',$,$,$,#27,#28);
#30=IFCDOOR('0000000000000000000030',$,$,$,$,$,$,$,$,$,$,$,$);
#31=IFCRELFILLSELEMENT('0000000000000000000031',$,$,$,#28,#30);
#32=IFCZONE('0000000000000000000032',$,$,$,$,$);
#33=IFCZONE('0000000000000000000033',$,$,$,$,$);
#34=IFCRELASSIGNSTOGROUP('0000000000000000000034',$,$,$,(#25),$,#32);
#35=IFCRELASSIGNSTOGROUP('0000000000000000000035',$,$,$,(#25),$,#33);
#36=IFCRELASSIGNSTOGROUP('0000000000000000000036',$,$,$,(#27),$,#32);
";

fn wholes(
    session: &EvidenceSession,
    local: &str,
    decomposition: Decomposition,
) -> Result<Vec<(String, Option<String>)>, DecompositionError> {
    session
        .service::<DecompositionServiceHandle>()
        .unwrap()
        .wholes(&id(local), decomposition)
        .map(|resolved| {
            assert!(resolved.evidence.exact);
            resolved
                .wholes
                .into_iter()
                .map(|whole| (whole.class, whole.predefined_type))
                .collect()
        })
}

fn classes(list: &[&str]) -> Vec<(String, Option<String>)> {
    list.iter()
        .map(|class| {
            let predefined = (*class == "IFCSPACE").then(|| "INTERNAL".to_owned());
            ((*class).to_owned(), predefined)
        })
        .collect()
}

#[test]
fn wholes_are_walked_per_relation() {
    let session = session("IFC4", STRUCTURE);
    assert_eq!(
        wholes(&session, "#25", Decomposition::Containment),
        Ok(classes(&["IFCSPACE"]))
    );
    // Containment is not aggregation.
    assert_eq!(
        wholes(&session, "#25", Decomposition::Aggregation),
        Ok(vec![])
    );
    assert_eq!(
        wholes(&session, "#23", Decomposition::Aggregation),
        Ok(classes(&["IFCBUILDINGSTOREY", "IFCBUILDING"]))
    );
    assert_eq!(
        wholes(&session, "#25", Decomposition::Any),
        Ok(classes(&["IFCSPACE", "IFCBUILDINGSTOREY", "IFCBUILDING"]))
    );
    // A door fills an opening that voids the wall; the opening voids it too.
    for part in ["#30", "#28"] {
        assert_eq!(
            wholes(&session, part, Decomposition::Voiding),
            Ok(classes(&["IFCWALL"])),
            "{part}"
        );
    }
    assert_eq!(
        wholes(&session, "#27", Decomposition::Grouping),
        Ok(classes(&["IFCZONE"]))
    );
}

#[test]
fn two_groups_are_ambiguous_not_a_choice() {
    let session = session("IFC4", STRUCTURE);
    assert!(matches!(
        wholes(&session, "#25", Decomposition::Grouping),
        Err(DecompositionError::Ambiguous(_))
    ));
}
