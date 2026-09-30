//! Streams written by `tests/java/Generate.java` with a JDK: every one reads
//! without trailing bytes and writes back byte for byte.

use std::path::PathBuf;

use axioval_java_stream::{
    Annotation, ClassData, Content, Elements, FieldType, FieldValue, Mutf8, ReadError, Record,
    RecordId, Stream, Value, read,
};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn fixture(name: &str) -> (Vec<u8>, Stream) {
    let bytes = std::fs::read(data_dir().join(format!("{name}.ser"))).expect("fixture exists");
    let stream = read(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        stream.to_bytes().expect("writes"),
        bytes,
        "{name} writes back byte for byte"
    );
    (bytes, stream)
}

/// The fixtures the Java program writes that this codec reads.
const READABLE: &[&str] = &[
    "annotated",
    "arrays",
    "collections",
    "custom",
    "enums",
    "exception",
    "externalizable",
    "graph",
    "hierarchy",
    "primitives",
    "protocol1",
    "proxy",
    "strings",
    "toplevel",
];

#[test]
fn every_fixture_round_trips_byte_for_byte() {
    let mut seen: Vec<String> = std::fs::read_dir(data_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".ser").map(str::to_owned))
        .collect();
    seen.sort();
    for name in &seen {
        if name == "protocol1-external" {
            continue;
        }
        assert!(READABLE.contains(&name.as_str()), "{name} is listed");
        fixture(name);
    }
    assert_eq!(
        seen.len(),
        READABLE.len() + 1,
        "every listed fixture exists"
    );
}

#[test]
fn a_re_read_stream_is_the_same_graph() {
    for name in READABLE {
        let (_, stream) = fixture(name);
        assert_eq!(read(&stream.to_bytes().unwrap()).unwrap(), stream, "{name}");
    }
}

#[test]
fn protocol_version_1_externalizable_data_is_refused() {
    let bytes = std::fs::read(data_dir().join("protocol1-external.ser")).unwrap();
    assert!(matches!(
        read(&bytes),
        Err(ReadError::ExternalWithoutBlockData { .. })
    ));
}

fn object(stream: &Stream, value: Value) -> RecordId {
    let id = value.record().expect("an object");
    assert!(
        matches!(stream.record(id), Some(Record::Object(_))),
        "{:?}",
        stream.record(id)
    );
    id
}

fn top(stream: &Stream, index: usize) -> Value {
    match &stream.contents[index] {
        Content::Value(value) => *value,
        other => panic!("content {index} is {other:?}"),
    }
}

fn string_field(stream: &Stream, id: RecordId, name: &str) -> Option<String> {
    match stream.field(id, name) {
        Some(FieldValue::Object(Value::Ref(s))) => {
            Some(stream.string(*s).unwrap().to_string_lossless().unwrap())
        }
        Some(FieldValue::Object(Value::Null)) => None,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn primitive_fields_decode_bit_for_bit() {
    let (_, stream) = fixture("primitives");
    let id = object(&stream, top(&stream, 0));
    assert_eq!(
        stream
            .class_name(match stream.record(id) {
                Some(Record::Object(o)) => o.desc,
                _ => unreachable!(),
            })
            .unwrap(),
        "Generate$Primitives"
    );
    assert_eq!(stream.field(id, "b"), Some(&FieldValue::Byte(-12)));
    assert_eq!(stream.field(id, "c"), Some(&FieldValue::Char(0xE9)));
    assert_eq!(
        stream.field(id, "d"),
        Some(&FieldValue::Double(std::f64::consts::PI))
    );
    let Some(FieldValue::Double(nan)) = stream.field(id, "nan") else {
        panic!()
    };
    // writeDouble canonicalizes NaN; payloads survive the codec (see hand.rs).
    assert_eq!(nan.to_bits(), 0x7ff8_0000_0000_0000);
    let Some(FieldValue::Float(zero)) = stream.field(id, "f") else {
        panic!()
    };
    assert_eq!(zero.to_bits(), (-0.0f32).to_bits());
    assert_eq!(stream.field(id, "i"), Some(&FieldValue::Int(i32::MIN)));
    assert_eq!(stream.field(id, "l"), Some(&FieldValue::Long(i64::MAX)));
    assert_eq!(stream.field(id, "s"), Some(&FieldValue::Short(i16::MIN)));
    assert_eq!(stream.field(id, "z"), Some(&FieldValue::Boolean(true)));
    assert_eq!(stream.field(id, "off"), Some(&FieldValue::Boolean(false)));
}

#[test]
fn strings_decode_from_modified_utf8() {
    let (_, stream) = fixture("strings");
    let id = object(&stream, top(&stream, 0));
    assert_eq!(string_field(&stream, id, "empty").unwrap(), "");
    assert_eq!(string_field(&stream, id, "ascii").unwrap(), "plain");
    assert_eq!(string_field(&stream, id, "nul").unwrap(), "a\0b");
    assert_eq!(string_field(&stream, id, "twoByte").unwrap(), "Grüße");
    assert_eq!(string_field(&stream, id, "threeByte").unwrap(), "€世");
    assert_eq!(
        string_field(&stream, id, "astral").unwrap(),
        "\u{1F600} \u{1D11E}"
    );
    assert_eq!(string_field(&stream, id, "absent"), None);
    let Some(FieldValue::Object(Value::Ref(surrogate))) = stream.field(id, "loneSurrogate") else {
        panic!()
    };
    assert_eq!(
        stream.string(*surrogate).unwrap().to_utf16(),
        [0x78, 0xD800, 0x79]
    );
    assert!(
        stream
            .string(*surrogate)
            .unwrap()
            .to_string_lossless()
            .is_err()
    );

    // The same string written twice is one record, referenced the second time.
    assert_eq!(top(&stream, 1), top(&stream, 2));
    let long_id = top(&stream, 3).record().unwrap();
    let Some(Record::String(long)) = stream.record(long_id) else {
        panic!()
    };
    assert!(long.long, "more than 65535 bytes is a TC_LONGSTRING");
    let text = long.value.to_string_lossless().unwrap();
    assert_eq!(text.chars().count(), 40_001);
    assert!(text.ends_with('\u{1F600}'));
}

#[test]
fn arrays_of_every_element_type() {
    let (_, stream) = fixture("arrays");
    let id = object(&stream, top(&stream, 0));
    let elements = |name: &str| match stream.field(id, name) {
        Some(FieldValue::Object(Value::Ref(array))) => match stream.record(*array) {
            Some(Record::Array(array)) => array.elements.clone(),
            other => panic!("{other:?}"),
        },
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(
        elements("bytes"),
        Elements::Byte(vec![0, 1, 0xFF, 0x7F, 0x80])
    );
    assert_eq!(elements("chars"), Elements::Char(vec![0x61, 0xE9, 0xFFFF]));
    assert_eq!(
        elements("doubles"),
        Elements::Double(vec![0.0, -1.5, f64::INFINITY])
    );
    assert_eq!(
        elements("floats"),
        Elements::Float(vec![1.25, f32::from_bits(1)])
    );
    assert_eq!(elements("ints"), Elements::Int(vec![1, -2, i32::MAX]));
    assert_eq!(elements("longs"), Elements::Long(vec![i64::MIN, 0]));
    assert_eq!(elements("shorts"), Elements::Short(vec![7, -7]));
    assert_eq!(
        elements("booleans"),
        Elements::Boolean(vec![true, false, true])
    );
    assert_eq!(elements("empty"), Elements::Int(vec![]));
    let Elements::Object(strings) = elements("strings") else {
        panic!()
    };
    assert_eq!(strings[1], Value::Null);
    assert_eq!(
        strings[0], strings[2],
        "the repeated string is a back-reference"
    );
    let Elements::Object(matrix) = elements("matrix") else {
        panic!()
    };
    assert_eq!(matrix.len(), 3);
    assert_eq!(matrix[2], Value::Null);

    // An Object[] that contains itself.
    let this = top(&stream, 1).record().unwrap();
    let Some(Record::Array(array)) = stream.record(this) else {
        panic!()
    };
    let Elements::Object(elements) = &array.elements else {
        panic!()
    };
    assert_eq!(elements[0], Value::Ref(this));
}

#[test]
fn back_references_and_cycles_share_records() {
    let (_, stream) = fixture("graph");
    let a = object(&stream, top(&stream, 0));
    let next = |id: RecordId| match stream.field(id, "next") {
        Some(FieldValue::Object(Value::Ref(next))) => *next,
        other => panic!("{other:?}"),
    };
    let b = next(a);
    let c = next(b);
    assert_eq!(next(c), a, "a -> b -> c -> a");
    assert_eq!(stream.field(a, "payload"), stream.field(c, "payload"));
    let this = object(&stream, top(&stream, 1));
    assert_eq!(next(this), this);
    assert_eq!(
        top(&stream, 2),
        Value::Ref(b),
        "a later top-level write refers back"
    );
    // A 64-link chain nests 64 objects deep.
    let mut link = top(&stream, 3).record().unwrap();
    let mut length = 1;
    while let Some(FieldValue::Object(Value::Ref(next))) = stream.field(link, "next") {
        link = *next;
        length += 1;
    }
    assert_eq!(length, 64);
}

#[test]
fn enum_constants_carry_their_names() {
    let (_, stream) = fixture("enums");
    let palette = object(&stream, top(&stream, 0));
    let constant = |value: &FieldValue| {
        let FieldValue::Object(Value::Ref(id)) = value else {
            panic!("{value:?}")
        };
        let Some(Record::Enum(constant)) = stream.record(*id) else {
            panic!()
        };
        (
            stream.class_name(constant.desc).unwrap().to_string(),
            stream.string(constant.name).unwrap().to_string(),
        )
    };
    assert_eq!(
        constant(stream.field(palette, "primary").unwrap()),
        ("Generate$Colour".into(), "RED".into())
    );
    // A constant with a body is written with its enum type's descriptor.
    assert_eq!(
        constant(stream.field(palette, "secondary").unwrap()),
        ("Generate$Colour".into(), "BLUE".into())
    );
    assert_eq!(
        stream.field(palette, "primary"),
        stream.field(palette, "again")
    );
    let Some(Record::Enum(red)) = stream.field(palette, "primary").and_then(|v| match v {
        FieldValue::Object(Value::Ref(id)) => stream.record(*id),
        _ => None,
    }) else {
        panic!()
    };
    let Some(Record::ClassDesc(desc)) = stream.record(red.desc) else {
        panic!()
    };
    assert!(desc.is_enum());
    let Some(Record::ClassDesc(base)) = stream.record(desc.superclass.unwrap()) else {
        panic!()
    };
    assert_eq!(base.name, "java.lang.Enum");
}

#[test]
fn custom_write_object_data_is_kept_in_order() {
    let (_, stream) = fixture("custom");
    let id = object(&stream, top(&stream, 0));
    assert_eq!(stream.field(id, "count"), Some(&FieldValue::Int(3)));
    let Some(Record::Object(custom)) = stream.record(id) else {
        panic!()
    };
    let [
        ClassData::Serial {
            custom: Some(items),
            ..
        },
    ] = custom.data.as_slice()
    else {
        panic!("{custom:?}")
    };
    // writeInt + writeUTF, then the int[], then 3000 bytes in 1024-byte
    // blocks, the string, and the trailing double.
    let shape: Vec<&str> = items
        .iter()
        .map(|item| match item {
            Annotation::BlockData(block) if block.long => "long",
            Annotation::BlockData(_) => "short",
            Annotation::Value(_) => "object",
        })
        .collect();
    assert_eq!(
        shape,
        ["short", "object", "long", "long", "long", "object", "short"]
    );
    let Annotation::BlockData(first) = &items[0] else {
        panic!()
    };
    assert_eq!(first.bytes[..4], 42i32.to_be_bytes());
    assert_eq!(&first.bytes[4..], b"\x00\x06custom");
}

#[test]
fn externalizable_objects_keep_their_block_data() {
    let (_, stream) = fixture("externalizable");
    let id = object(&stream, top(&stream, 0));
    let Some(Record::Object(ext)) = stream.record(id) else {
        panic!()
    };
    let Some(Record::ClassDesc(desc)) = stream.record(ext.desc) else {
        panic!()
    };
    assert!(desc.is_externalizable() && desc.has_block_data());
    let [ClassData::External(items)] = ext.data.as_slice() else {
        panic!()
    };
    assert_eq!(items.len(), 4, "int, label, node, long: {items:?}");
}

#[test]
fn proxy_descriptors_name_their_interfaces() {
    let (_, stream) = fixture("proxy");
    let id = object(&stream, top(&stream, 0));
    let Some(Record::Object(proxy)) = stream.record(id) else {
        panic!()
    };
    let Some(Record::ProxyClassDesc(desc)) = stream.record(proxy.desc) else {
        panic!()
    };
    assert_eq!(desc.interfaces, [Mutf8::encode("Generate$Greeter")]);
    assert_eq!(
        stream.class_name(desc.superclass.unwrap()).unwrap(),
        "java.lang.reflect.Proxy"
    );
    // The handler lives in java.lang.reflect.Proxy's field `h`.
    let Some(FieldValue::Object(Value::Ref(handler))) = stream.field(id, "h") else {
        panic!()
    };
    assert_eq!(string_field(&stream, *handler, "prefix").unwrap(), "hello ");
    assert_eq!(top(&stream, 2), Value::Ref(id));
}

#[test]
fn subclass_chains_have_one_data_entry_per_serializable_class() {
    let (_, stream) = fixture("hierarchy");
    let id = object(&stream, top(&stream, 0));
    let Some(Record::Object(dog)) = stream.record(id) else {
        panic!()
    };
    let chain = stream.class_chain(dog.desc).unwrap();
    let names: Vec<String> = chain
        .iter()
        .map(|d| stream.class_name(*d).unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        ["Generate$Animal", "Generate$Mammal", "Generate$Dog"]
    );
    assert_eq!(dog.data.len(), 3);
    assert!(
        matches!(
            &dog.data[1],
            ClassData::Serial {
                custom: Some(_),
                ..
            }
        ),
        "Mammal has writeObject"
    );
    assert_eq!(string_field(&stream, id, "name").unwrap(), "Fido");
    assert_eq!(string_field(&stream, id, "kind").unwrap(), "animal");
    assert_eq!(stream.field(id, "legs"), Some(&FieldValue::Int(4)));
    let Some(Record::ClassDesc(dog_desc)) = stream.record(dog.desc) else {
        panic!()
    };
    let friend = dog_desc.fields.iter().find(|f| f.name == "friend").unwrap();
    let FieldType::Object(Value::Ref(ty)) = friend.ty else {
        panic!()
    };
    assert_eq!(stream.string(ty).unwrap(), "LGenerate$Animal;");
}

#[test]
fn class_annotations_are_kept() {
    let (_, stream) = fixture("annotated");
    let annotated = stream
        .records
        .iter()
        .filter_map(|record| match record {
            Record::ClassDesc(desc) if !desc.annotation.is_empty() => Some(desc.annotation.len()),
            Record::ProxyClassDesc(desc) if !desc.annotation.is_empty() => {
                Some(desc.annotation.len())
            }
            _ => None,
        })
        .count();
    assert!(annotated >= 4, "{annotated}");
}

#[test]
fn top_level_block_data_resets_classes_and_descriptors() {
    let (_, stream) = fixture("toplevel");
    let kinds: Vec<&str> = stream
        .contents
        .iter()
        .map(|content| match content {
            Content::Value(Value::Null) => "null",
            Content::Value(Value::Ref(id)) => stream.record(*id).unwrap().kind(),
            Content::BlockData(_) => "block",
            Content::Reset => "reset",
            Content::Exception(_) => "exception",
        })
        .collect();
    assert_eq!(
        kinds[..12],
        [
            "block",
            "object",
            "object",
            "reset",
            "object",
            "class",
            "class",
            "class",
            "class",
            "class descriptor",
            "object",
            "object"
        ]
    );
    assert_eq!(kinds[12], "null");
    assert!(kinds[13..].iter().all(|kind| *kind == "block"));
    // Before the reset the second write refers back; after it the object is
    // written anew, and writeUnshared never refers back.
    assert_eq!(top(&stream, 1), top(&stream, 2));
    assert_ne!(top(&stream, 2), top(&stream, 4));
    assert_ne!(top(&stream, 10), top(&stream, 11));
}

#[test]
fn an_aborted_write_keeps_its_exception() {
    let (_, stream) = fixture("exception");
    let [
        Content::Value(_),
        Content::Exception(exception),
        Content::Value(after),
    ] = stream.contents.as_slice()
    else {
        panic!("{:?}", stream.contents)
    };
    // The aborted object got as far as its first field.
    assert_eq!(exception.aborted[0], axioval_java_stream::TC_OBJECT);
    let throwable = exception.throwable.record().unwrap();
    let Some(Record::Object(throwable)) = stream.record(throwable) else {
        panic!()
    };
    assert_eq!(
        stream.class_name(throwable.desc).unwrap(),
        "java.io.NotSerializableException"
    );
    assert_eq!(stream.string(after.record().unwrap()).unwrap(), "after");
}
