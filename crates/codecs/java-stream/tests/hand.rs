//! Streams built by hand from the specification's grammar, and graphs built
//! with the builder API.

use axioval_java_stream::{
    Annotation, Array, BlockData, ClassData, ClassDesc, Content, Elements, EnumConstant, Exception,
    FieldType, FieldValue, JavaString, Mutf8, Object, ProxyClassDesc, Record, RecordId,
    SC_BLOCK_DATA, SC_ENUM, SC_EXTERNALIZABLE, SC_SERIALIZABLE, SC_WRITE_METHOD, Stream, Value,
    WriteError, read,
};

/// The two-element linked list of the specification's example (§6.4.2).
const SPEC_EXAMPLE: &[u8] = &[
    0xAC, 0xED, 0x00, 0x05, 0x73, 0x72, 0x00, 0x04, 0x4C, 0x69, 0x73, 0x74, 0x69, 0xC8, 0x8A,
    0x15, //
    0x40, 0x16, 0xAE, 0x68, 0x02, 0x00, 0x02, 0x49, 0x00, 0x05, 0x76, 0x61, 0x6C, 0x75, 0x65,
    0x4C, //
    0x00, 0x04, 0x6E, 0x65, 0x78, 0x74, 0x74, 0x00, 0x06, 0x4C, 0x4C, 0x69, 0x73, 0x74, 0x3B,
    0x78, //
    0x70, 0x00, 0x00, 0x00, 0x11, 0x73, 0x71, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x13,
    0x70, //
    0x71, 0x00, 0x7E, 0x00, 0x03,
];

fn round_trip(bytes: &[u8]) -> Stream {
    let stream = read(bytes).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(stream.to_bytes().unwrap(), bytes);
    stream
}

fn serial(values: Vec<FieldValue>) -> ClassData {
    ClassData::Serial {
        values,
        custom: None,
    }
}

#[test]
fn the_specification_example_round_trips() {
    let stream = round_trip(SPEC_EXAMPLE);
    let [
        Content::Value(Value::Ref(first)),
        Content::Value(Value::Ref(second)),
    ] = stream.contents[..]
    else {
        panic!("{:?}", stream.contents)
    };
    assert_eq!(stream.field(first, "value"), Some(&FieldValue::Int(17)));
    assert_eq!(
        stream.field(first, "next"),
        Some(&FieldValue::Object(Value::Ref(second)))
    );
    assert_eq!(stream.field(second, "value"), Some(&FieldValue::Int(19)));
    assert_eq!(
        stream.field(second, "next"),
        Some(&FieldValue::Object(Value::Null))
    );
}

#[test]
fn the_builder_writes_the_specification_example() {
    let mut stream = Stream::new();
    let ty = stream.add_string("LList;");
    let desc = stream.add_class_desc(
        ClassDesc::new("List", 0x69C8_8A15_4016_AE68, SC_SERIALIZABLE)
            .with_field("value", FieldType::Int)
            .with_field("next", FieldType::Object(Value::Ref(ty))),
    );
    let second = stream.add_object(
        desc,
        vec![serial(vec![
            FieldValue::Int(19),
            FieldValue::Object(Value::Null),
        ])],
    );
    let first = stream.add_object(
        desc,
        vec![serial(vec![
            FieldValue::Int(17),
            FieldValue::Object(second.into()),
        ])],
    );
    stream.push_value(first);
    stream.push_value(second);
    // Records were added in another order than the stream defines them;
    // the writer assigns handles in grammar order all the same.
    assert_eq!(stream.to_bytes().unwrap(), SPEC_EXAMPLE);
}

#[test]
fn the_builder_writes_cycles_and_every_record_kind() {
    let mut stream = Stream::new();
    let node_ty = stream.add_string("LNode;");
    let base = stream.add_class_desc(
        ClassDesc::new("Base", 7, SC_SERIALIZABLE).with_field("flag", FieldType::Boolean),
    );
    let desc = stream.add_class_desc(
        ClassDesc::new("Node", 1, SC_SERIALIZABLE | SC_WRITE_METHOD)
            .with_field("next", FieldType::Object(node_ty.into()))
            .with_superclass(base),
    );
    let node = stream.add_object(desc, Vec::new());
    let Some(Record::Object(object)) = stream.record_mut(node) else {
        unreachable!()
    };
    object.data = vec![
        serial(vec![FieldValue::Boolean(true)]),
        ClassData::Serial {
            values: vec![FieldValue::Object(node.into())],
            custom: Some(vec![Annotation::BlockData(BlockData::new(vec![1, 2, 3]))]),
        },
    ];
    let array_desc = stream.add_class_desc(ClassDesc::new("[D", 0, SC_SERIALIZABLE));
    let nan = f64::from_bits(0x7FF8_0000_DEAD_BEEF);
    let array = stream.add(Record::Array(Array {
        desc: array_desc,
        elements: Elements::Double(vec![nan, -0.0]),
    }));
    let enum_base = stream.add_class_desc(ClassDesc::new(
        "java.lang.Enum",
        0,
        SC_SERIALIZABLE | SC_ENUM,
    ));
    let enum_desc = stream.add_class_desc(
        ClassDesc::new("Colour", 0, SC_SERIALIZABLE | SC_ENUM).with_superclass(enum_base),
    );
    let red = stream.add_string("RED");
    let constant = stream.add(Record::Enum(EnumConstant {
        desc: enum_desc,
        name: red,
    }));
    let proxy = stream.add(Record::ProxyClassDesc(ProxyClassDesc {
        interfaces: vec![Mutf8::encode("Api")],
        annotation: Vec::new(),
        superclass: None,
    }));
    let ext_desc =
        stream.add_class_desc(ClassDesc::new("Ext", 3, SC_EXTERNALIZABLE | SC_BLOCK_DATA));
    let ext = stream.add_object(
        ext_desc,
        vec![ClassData::External(vec![Annotation::Value(node.into())])],
    );
    let class = stream.add(Record::Class(axioval_java_stream::ClassRecord {
        desc: proxy,
    }));
    for id in [node, array, constant, ext, class, red] {
        stream.push_value(id);
    }
    stream.push(Content::BlockData(BlockData::new(vec![0; 300])));
    stream.push(Content::Reset);
    let fresh = stream.add_string("after reset");
    stream.push_value(fresh);

    let bytes = stream.to_bytes().unwrap();
    let back = round_trip(&bytes);
    assert_eq!(back.records.len(), stream.records.len());
    let [
        Content::Value(Value::Ref(node)),
        Content::Value(Value::Ref(array)),
        ..,
    ] = back.contents[..]
    else {
        panic!()
    };
    assert_eq!(
        back.field(node, "next"),
        Some(&FieldValue::Object(Value::Ref(node)))
    );
    assert_eq!(back.field(node, "flag"), Some(&FieldValue::Boolean(true)));
    let Some(Record::Array(array)) = back.record(array) else {
        panic!()
    };
    let Elements::Double(values) = &array.elements else {
        panic!()
    };
    assert_eq!(
        values[0].to_bits(),
        0x7FF8_0000_DEAD_BEEF,
        "NaN payloads survive"
    );
    assert!(matches!(
        &back.contents[6],
        Content::BlockData(BlockData { long: true, .. })
    ));
}

#[test]
fn non_canonical_forms_write_back_unchanged() {
    let mut bytes = vec![0xAC, 0xED, 0x00, 0x05];
    // Three bytes in a TC_BLOCKDATALONG.
    bytes.extend([0x7A, 0, 0, 0, 3, 1, 2, 3]);
    // A short string in a TC_LONGSTRING, with a non-shortest 'A' (C1 81).
    bytes.extend([0x7C, 0, 0, 0, 0, 0, 0, 0, 3, b'x', 0xC1, 0x81]);
    // An empty TC_BLOCKDATA.
    bytes.extend([0x77, 0]);
    let stream = round_trip(&bytes);
    let [
        Content::BlockData(block),
        Content::Value(Value::Ref(string)),
        Content::BlockData(empty),
    ] = &stream.contents[..]
    else {
        panic!()
    };
    assert!(block.long);
    assert!(!empty.long && empty.bytes.is_empty());
    assert_eq!(
        stream
            .string(*string)
            .unwrap()
            .to_string_lossless()
            .unwrap(),
        "xA"
    );
}

#[test]
fn a_top_level_exception_resets_handles() {
    let mut bytes = vec![0xAC, 0xED, 0x00, 0x05];
    bytes.extend([0x74, 0, 1, b'a']); // handle 0: "a"
    bytes.extend([0x7B]); // TC_EXCEPTION
    bytes.extend([0x74, 0, 1, b'e']); // the "throwable", handle 0 again
    bytes.extend([0x71, 0, 0x7E, 0, 0]); // references nothing: reset after the exception
    assert!(read(&bytes).is_err());
    bytes.truncate(bytes.len() - 5);
    bytes.extend([0x74, 0, 1, b'b', 0x71, 0, 0x7E, 0, 0]); // "b" is handle 0 once more
    let stream = round_trip(&bytes);
    assert!(
        matches!(&stream.contents[1], Content::Exception(Exception { aborted, .. }) if aborted.is_empty())
    );
    assert_eq!(stream.contents[2], stream.contents[3]);
}

#[test]
fn a_nested_exception_aborts_the_top_level_object() {
    let mut bytes = vec![0xAC, 0xED, 0x00, 0x05];
    // Object[] with two elements; the second is aborted by TC_EXCEPTION.
    bytes.extend([
        0x75, 0x72, 0, 2, b'[', b'L', 0, 0, 0, 0, 0, 0, 0, 1, 0x02, 0, 0, 0x78, 0x70,
    ]);
    bytes.extend([0, 0, 0, 2, 0x70]);
    let aborted = bytes[4..].to_vec();
    bytes.extend([0x7B, 0x74, 0, 3, b'o', b'o', b'p']);
    let stream = round_trip(&bytes);
    let [Content::Exception(exception)] = &stream.contents[..] else {
        panic!("{:?}", stream.contents)
    };
    assert_eq!(exception.aborted, aborted);
    // The aborted array's records are gone; only the exception's remain.
    assert_eq!(stream.records.len(), 1);
}

#[test]
fn builder_misuse_is_refused_with_typed_errors() {
    let mut stream = Stream::new();
    stream.push_value(RecordId(9));
    assert_eq!(
        stream.to_bytes(),
        Err(WriteError::UnknownRecord(RecordId(9)))
    );

    let mut stream = Stream::new();
    let s = stream.add_string("s");
    let object = stream.add_object(s, Vec::new());
    stream.push_value(object);
    assert!(matches!(
        stream.to_bytes(),
        Err(WriteError::WrongRecord {
            expected: "class descriptor",
            ..
        })
    ));

    let mut stream = Stream::new();
    let desc = stream
        .add_class_desc(ClassDesc::new("P", 1, SC_SERIALIZABLE).with_field("x", FieldType::Int));
    let object = stream.add_object(desc, vec![serial(vec![FieldValue::Long(1)])]);
    stream.push_value(object);
    assert!(matches!(
        stream.to_bytes(),
        Err(WriteError::ClassDataMismatch { .. })
    ));

    let mut stream = Stream::new();
    let desc = stream.add_class_desc(ClassDesc::new("[I", 1, SC_SERIALIZABLE));
    let array = stream.add(Record::Array(Array {
        desc,
        elements: Elements::Long(vec![1]),
    }));
    stream.push_value(array);
    assert_eq!(stream.to_bytes(), Err(WriteError::ArrayMismatch(array)));

    let mut stream = Stream::new();
    let s = stream.add_string("s");
    stream.push_value(s);
    stream.push(Content::Reset);
    stream.push_value(s);
    assert_eq!(stream.to_bytes(), Err(WriteError::StaleReference(s)));

    let mut stream = Stream::new();
    let a = stream.add_class_desc(ClassDesc::new("A", 1, SC_SERIALIZABLE));
    let b = stream.add_class_desc(ClassDesc::new("B", 1, SC_SERIALIZABLE).with_superclass(a));
    let Some(Record::ClassDesc(first)) = stream.record_mut(a) else {
        unreachable!()
    };
    first.superclass = Some(b);
    let class = stream.add(Record::Class(axioval_java_stream::ClassRecord { desc: b }));
    stream.push_value(class);
    assert_eq!(stream.to_bytes(), Err(WriteError::IncompleteClassDesc(b)));

    let mut stream = Stream::new();
    stream.push(Content::BlockData(BlockData {
        bytes: vec![0; 256],
        long: false,
    }));
    assert!(matches!(
        stream.to_bytes(),
        Err(WriteError::TooLong {
            what: "block data",
            ..
        })
    ));

    let mut stream = Stream::new();
    let long = stream.add(Record::String(JavaString {
        value: Mutf8::encode(&"x".repeat(70_000)),
        long: false,
    }));
    stream.push_value(long);
    assert!(matches!(
        stream.to_bytes(),
        Err(WriteError::TooLong { what: "string", .. })
    ));

    // An object of a class inside that class's own annotation.
    let mut stream = Stream::new();
    let desc = stream.add_class_desc(ClassDesc::new("Self", 1, SC_SERIALIZABLE));
    let inner = stream.add_object(desc, vec![serial(Vec::new())]);
    let Some(Record::ClassDesc(self_desc)) = stream.record_mut(desc) else {
        unreachable!()
    };
    self_desc.annotation.push(Annotation::Value(inner.into()));
    let outer = stream.add_object(desc, vec![serial(Vec::new())]);
    stream.push_value(outer);
    assert_eq!(
        stream.to_bytes(),
        Err(WriteError::IncompleteClassDesc(desc))
    );

    let mut stream = Stream::new();
    let desc = stream.add_class_desc(ClassDesc::new("Ext", 1, SC_EXTERNALIZABLE));
    let ext = stream.add(Record::Object(Object {
        desc,
        data: vec![ClassData::External(Vec::new())],
    }));
    stream.push_value(ext);
    assert!(matches!(
        stream.to_bytes(),
        Err(WriteError::ClassDataMismatch { .. })
    ));
}

#[test]
fn a_graph_deeper_than_the_limit_is_not_written() {
    let mut stream = Stream::new();
    let desc = stream.add_class_desc(ClassDesc::new("[Ljava.lang.Object;", 1, SC_SERIALIZABLE));
    let mut inner = Value::Null;
    for _ in 0..1000 {
        inner = stream
            .add(Record::Array(Array {
                desc,
                elements: Elements::Object(vec![inner]),
            }))
            .into();
    }
    stream.push(Content::Value(inner));
    assert_eq!(stream.to_bytes(), Err(WriteError::TooDeep));
}
