//! The graph a stream holds, and the builder methods that assemble one.
//!
//! Every record that takes a handle on the wire (`TC_OBJECT`, `TC_CLASS`,
//! `TC_ARRAY`, `TC_STRING`, `TC_LONGSTRING`, `TC_ENUM`, `TC_CLASSDESC`,
//! `TC_PROXYCLASSDESC`) is one [`Record`] in [`Stream::records`], in the order
//! the stream defines them. A position that holds an object refers to a
//! record by [`RecordId`]; the writer defines a record inline at the first
//! position it meets it and writes `TC_REFERENCE` at every later one, so
//! handles are never stored and always come out in grammar order.

use crate::Mutf8;
use crate::protocol::{
    SC_BLOCK_DATA, SC_ENUM, SC_EXTERNALIZABLE, SC_SERIALIZABLE, SC_WRITE_METHOD,
};

/// Index of a record in [`Stream::records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(pub usize);

/// A position that holds an object: `TC_NULL`, or a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Value {
    /// `TC_NULL`.
    Null,
    /// A record, defined here if the writer has not met it yet, otherwise a
    /// `TC_REFERENCE` to its handle.
    Ref(RecordId),
}

impl Value {
    /// The record this value refers to, if any.
    pub fn record(self) -> Option<RecordId> {
        match self {
            Self::Null => None,
            Self::Ref(id) => Some(id),
        }
    }
}

impl From<RecordId> for Value {
    fn from(id: RecordId) -> Self {
        Self::Ref(id)
    }
}

/// A whole stream: its top-level contents and the records they define.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stream {
    /// Top-level contents, in stream order.
    pub contents: Vec<Content>,
    /// Every record the stream defines, in definition order.
    pub records: Vec<Record>,
}

/// One top-level item of a stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// An object written with `writeObject` or `writeUnshared`.
    Value(Value),
    /// Primitive data written outside any object.
    BlockData(BlockData),
    /// `TC_RESET`: the writer forgot every handle.
    Reset,
    /// `TC_EXCEPTION`: the writer aborted an object and wrote the exception.
    Exception(Exception),
}

/// An aborted write: `TC_EXCEPTION reset (Throwable)object reset`.
///
/// The writer resets its handles before and after the exception, so what it
/// had written of the aborted object is meaningless to a reader and is kept
/// as opaque bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Exception {
    /// The bytes of the aborted top-level object, up to `TC_EXCEPTION`.
    pub aborted: Vec<u8>,
    /// The exception object, written in a fresh handle table.
    pub throwable: Value,
}

/// One block-data record: `TC_BLOCKDATA` or `TC_BLOCKDATALONG`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockData {
    /// The raw bytes.
    pub bytes: Vec<u8>,
    /// Written with `TC_BLOCKDATALONG` (a four-byte length). Java uses it
    /// for more than 255 bytes; kept so any stream writes back unchanged.
    pub long: bool,
}

impl BlockData {
    /// Block data in the form Java would write it.
    pub fn new(bytes: Vec<u8>) -> Self {
        let long = bytes.len() > 0xFF;
        Self { bytes, long }
    }
}

/// An item of an annotation or of custom `writeObject` data, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    /// Primitive data.
    BlockData(BlockData),
    /// An object.
    Value(Value),
}

/// A record that takes a handle.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// `TC_CLASSDESC`.
    ClassDesc(ClassDesc),
    /// `TC_PROXYCLASSDESC`.
    ProxyClassDesc(ProxyClassDesc),
    /// `TC_CLASS`: a `java.lang.Class` object.
    Class(ClassRecord),
    /// `TC_OBJECT`.
    Object(Object),
    /// `TC_ARRAY`.
    Array(Array),
    /// `TC_STRING` or `TC_LONGSTRING`.
    String(JavaString),
    /// `TC_ENUM`.
    Enum(EnumConstant),
}

impl Record {
    /// The kind of record, as the stream names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ClassDesc(_) => "class descriptor",
            Self::ProxyClassDesc(_) => "proxy class descriptor",
            Self::Class(_) => "class",
            Self::Object(_) => "object",
            Self::Array(_) => "array",
            Self::String(_) => "string",
            Self::Enum(_) => "enum constant",
        }
    }
}

/// A class descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDesc {
    /// The class name, in `Class.getName` form.
    pub name: Mutf8,
    /// The `serialVersionUID`.
    pub serial_version_uid: i64,
    /// The `SC_*` flags byte, as written.
    pub flags: u8,
    /// Serializable fields, in descriptor order.
    pub fields: Vec<FieldDesc>,
    /// What `annotateClass` wrote.
    pub annotation: Vec<Annotation>,
    /// The superclass descriptor: `None` for `TC_NULL`.
    pub superclass: Option<RecordId>,
}

impl ClassDesc {
    /// A descriptor without fields, annotation or superclass.
    pub fn new(name: &str, serial_version_uid: i64, flags: u8) -> Self {
        Self {
            name: Mutf8::encode(name),
            serial_version_uid,
            flags,
            fields: Vec::new(),
            annotation: Vec::new(),
            superclass: None,
        }
    }

    /// Adds a field.
    #[must_use]
    pub fn with_field(mut self, name: &str, ty: FieldType) -> Self {
        self.fields.push(FieldDesc {
            name: Mutf8::encode(name),
            ty,
        });
        self
    }

    /// Sets the superclass descriptor.
    #[must_use]
    pub fn with_superclass(mut self, superclass: RecordId) -> Self {
        self.superclass = Some(superclass);
        self
    }

    /// `SC_SERIALIZABLE`.
    pub fn is_serializable(&self) -> bool {
        self.flags & SC_SERIALIZABLE != 0
    }

    /// `SC_WRITE_METHOD`: each object carries custom data for this class.
    pub fn has_write_method(&self) -> bool {
        self.flags & SC_WRITE_METHOD != 0
    }

    /// `SC_EXTERNALIZABLE`.
    pub fn is_externalizable(&self) -> bool {
        self.flags & SC_EXTERNALIZABLE != 0
    }

    /// `SC_BLOCK_DATA`: externalizable data is written as block data.
    pub fn has_block_data(&self) -> bool {
        self.flags & SC_BLOCK_DATA != 0
    }

    /// `SC_ENUM`.
    pub fn is_enum(&self) -> bool {
        self.flags & SC_ENUM != 0
    }
}

/// A proxy class descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyClassDesc {
    /// The interfaces the proxy class implements, in order.
    pub interfaces: Vec<Mutf8>,
    /// What `annotateProxyClass` wrote.
    pub annotation: Vec<Annotation>,
    /// The superclass descriptor, usually `java.lang.reflect.Proxy`.
    pub superclass: Option<RecordId>,
}

/// A field of a class descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDesc {
    /// The field name.
    pub name: Mutf8,
    /// The field type.
    pub ty: FieldType,
}

/// A field type: a primitive, or an object with its type string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// `B`.
    Byte,
    /// `C`.
    Char,
    /// `D`.
    Double,
    /// `F`.
    Float,
    /// `I`.
    Int,
    /// `J`.
    Long,
    /// `S`.
    Short,
    /// `Z`.
    Boolean,
    /// `[`: an array; the value is its type string (`[I`), a string record.
    Array(Value),
    /// `L`: an object; the value is its type string (`Ljava/lang/String;`).
    Object(Value),
}

impl FieldType {
    /// The primitive for a type code, or `None` for `[`, `L` and others.
    pub fn primitive(code: u8) -> Option<Self> {
        Some(match code {
            b'B' => Self::Byte,
            b'C' => Self::Char,
            b'D' => Self::Double,
            b'F' => Self::Float,
            b'I' => Self::Int,
            b'J' => Self::Long,
            b'S' => Self::Short,
            b'Z' => Self::Boolean,
            _ => return None,
        })
    }

    /// The type code.
    pub fn code(self) -> u8 {
        match self {
            Self::Byte => b'B',
            Self::Char => b'C',
            Self::Double => b'D',
            Self::Float => b'F',
            Self::Int => b'I',
            Self::Long => b'J',
            Self::Short => b'S',
            Self::Boolean => b'Z',
            Self::Array(_) => b'[',
            Self::Object(_) => b'L',
        }
    }

    /// Whether values of this type are objects.
    pub fn is_object(self) -> bool {
        matches!(self, Self::Array(_) | Self::Object(_))
    }
}

/// A field value. Floating-point values compare bit for bit, so a graph
/// equals itself even where it holds NaN.
#[derive(Debug, Clone, Copy)]
pub enum FieldValue {
    /// `byte`.
    Byte(i8),
    /// `char`, a UTF-16 code unit.
    Char(u16),
    /// `double`, bit for bit.
    Double(f64),
    /// `float`, bit for bit.
    Float(f32),
    /// `int`.
    Int(i32),
    /// `long`.
    Long(i64),
    /// `short`.
    Short(i16),
    /// `boolean`.
    Boolean(bool),
    /// An object or array field.
    Object(Value),
}

impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Double(a), Self::Double(b)) => a.to_bits() == b.to_bits(),
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Byte(a), Self::Byte(b)) => a == b,
            (Self::Char(a), Self::Char(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Long(a), Self::Long(b)) => a == b,
            (Self::Short(a), Self::Short(b)) => a == b,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl FieldValue {
    /// Whether this value fits a field of `ty`.
    pub fn fits(&self, ty: FieldType) -> bool {
        matches!(
            (self, ty),
            (Self::Byte(_), FieldType::Byte)
                | (Self::Char(_), FieldType::Char)
                | (Self::Double(_), FieldType::Double)
                | (Self::Float(_), FieldType::Float)
                | (Self::Int(_), FieldType::Int)
                | (Self::Long(_), FieldType::Long)
                | (Self::Short(_), FieldType::Short)
                | (Self::Boolean(_), FieldType::Boolean)
                | (Self::Object(_), FieldType::Array(_) | FieldType::Object(_))
        )
    }
}

/// A `java.lang.Class` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassRecord {
    /// Its class descriptor.
    pub desc: RecordId,
}

/// An ordinary object.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// Its class descriptor.
    pub desc: RecordId,
    /// Its class data: for a serializable object one entry per descriptor
    /// of its superclass chain, the topmost superclass first; for an
    /// externalizable object one [`ClassData::External`].
    pub data: Vec<ClassData>,
}

/// The data one class of an object wrote.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassData {
    /// Default field values, then custom `writeObject` data if the class
    /// descriptor has `SC_WRITE_METHOD` (and only then).
    Serial {
        /// One value per field of the descriptor, in its order.
        values: Vec<FieldValue>,
        /// Custom data up to `TC_ENDBLOCKDATA`.
        custom: Option<Vec<Annotation>>,
    },
    /// What `writeExternal` wrote, in block-data mode, up to
    /// `TC_ENDBLOCKDATA`.
    External(Vec<Annotation>),
}

/// An array.
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    /// Its class descriptor, whose name gives the element type.
    pub desc: RecordId,
    /// Its elements.
    pub elements: Elements,
}

/// The elements of an array. Floating-point elements compare bit for bit.
#[derive(Debug, Clone)]
pub enum Elements {
    /// `byte[]`, as raw bytes.
    Byte(Vec<u8>),
    /// `char[]`.
    Char(Vec<u16>),
    /// `double[]`.
    Double(Vec<f64>),
    /// `float[]`.
    Float(Vec<f32>),
    /// `int[]`.
    Int(Vec<i32>),
    /// `long[]`.
    Long(Vec<i64>),
    /// `short[]`.
    Short(Vec<i16>),
    /// `boolean[]`.
    Boolean(Vec<bool>),
    /// An array of objects or arrays.
    Object(Vec<Value>),
}

impl PartialEq for Elements {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Double(a), Self::Double(b)) => a
                .iter()
                .map(|v| v.to_bits())
                .eq(b.iter().map(|v| v.to_bits())),
            (Self::Float(a), Self::Float(b)) => a
                .iter()
                .map(|v| v.to_bits())
                .eq(b.iter().map(|v| v.to_bits())),
            (Self::Byte(a), Self::Byte(b)) => a == b,
            (Self::Char(a), Self::Char(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Long(a), Self::Long(b)) => a == b,
            (Self::Short(a), Self::Short(b)) => a == b,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl Elements {
    /// The number of elements.
    pub fn len(&self) -> usize {
        match self {
            Self::Byte(v) => v.len(),
            Self::Char(v) => v.len(),
            Self::Double(v) => v.len(),
            Self::Float(v) => v.len(),
            Self::Int(v) => v.len(),
            Self::Long(v) => v.len(),
            Self::Short(v) => v.len(),
            Self::Boolean(v) => v.len(),
            Self::Object(v) => v.len(),
        }
    }

    /// Whether the array is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The component type code these elements are written as.
    pub(crate) fn code(&self) -> u8 {
        match self {
            Self::Byte(_) => b'B',
            Self::Char(_) => b'C',
            Self::Double(_) => b'D',
            Self::Float(_) => b'F',
            Self::Int(_) => b'I',
            Self::Long(_) => b'J',
            Self::Short(_) => b'S',
            Self::Boolean(_) => b'Z',
            Self::Object(_) => b'L',
        }
    }
}

/// A string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaString {
    /// The characters.
    pub value: Mutf8,
    /// Written with `TC_LONGSTRING` (an eight-byte length). Java uses it for
    /// more than 65535 bytes; kept so any stream writes back unchanged.
    pub long: bool,
}

impl JavaString {
    /// A string in the form Java would write it.
    pub fn new(value: Mutf8) -> Self {
        let long = value.len() > 0xFFFF;
        Self { value, long }
    }
}

/// An enum constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumConstant {
    /// The enum type's class descriptor.
    pub desc: RecordId,
    /// The constant's name, a string record.
    pub name: RecordId,
}

impl Stream {
    /// An empty stream: just the magic number and version.
    pub fn new() -> Self {
        Self::default()
    }

    /// The record `id` refers to.
    pub fn record(&self, id: RecordId) -> Option<&Record> {
        self.records.get(id.0)
    }

    /// The record `id` refers to, for patching a graph with cycles.
    pub fn record_mut(&mut self, id: RecordId) -> Option<&mut Record> {
        self.records.get_mut(id.0)
    }

    /// Adds a record and returns its id. It is written at the first
    /// position that refers to it.
    pub fn add(&mut self, record: Record) -> RecordId {
        self.records.push(record);
        RecordId(self.records.len() - 1)
    }

    /// Appends a top-level item.
    pub fn push(&mut self, content: Content) {
        self.contents.push(content);
    }

    /// Appends a top-level object.
    pub fn push_value(&mut self, value: impl Into<Value>) {
        self.contents.push(Content::Value(value.into()));
    }

    /// Adds a string record in the form Java would write it.
    pub fn add_string(&mut self, text: &str) -> RecordId {
        self.add(Record::String(JavaString::new(Mutf8::encode(text))))
    }

    /// Adds a class descriptor.
    pub fn add_class_desc(&mut self, desc: ClassDesc) -> RecordId {
        self.add(Record::ClassDesc(desc))
    }

    /// Adds an object.
    pub fn add_object(&mut self, desc: RecordId, data: Vec<ClassData>) -> RecordId {
        self.add(Record::Object(Object { desc, data }))
    }

    /// The string a record holds, if it is a string record.
    pub fn string(&self, id: RecordId) -> Option<&Mutf8> {
        match self.record(id)? {
            Record::String(string) => Some(&string.value),
            _ => None,
        }
    }

    /// The name of the class a descriptor describes; `None` for a proxy.
    pub fn class_name(&self, desc: RecordId) -> Option<&Mutf8> {
        match self.record(desc)? {
            Record::ClassDesc(desc) => Some(&desc.name),
            _ => None,
        }
    }

    /// The descriptor chain of `desc`, the topmost superclass first.
    ///
    /// # Errors
    ///
    /// The first id on the chain that is no descriptor, or that closes a
    /// cycle.
    pub fn class_chain(&self, desc: RecordId) -> Result<Vec<RecordId>, RecordId> {
        class_chain(&self.records, desc)
    }

    /// The value of the field `name` of an object, searched from the
    /// object's own class up its superclasses.
    pub fn field(&self, object: RecordId, name: &str) -> Option<&FieldValue> {
        let Some(Record::Object(object)) = self.record(object) else {
            return None;
        };
        let chain = self.class_chain(object.desc).ok()?;
        for (desc, data) in chain.iter().zip(&object.data).rev() {
            let (Some(Record::ClassDesc(desc)), ClassData::Serial { values, .. }) =
                (self.record(*desc), data)
            else {
                continue;
            };
            if let Some(index) = desc.fields.iter().position(|field| field.name == name) {
                return values.get(index);
            }
        }
        None
    }
}

/// The descriptor chain of `desc` in `records`, the topmost superclass first.
pub(crate) fn class_chain(records: &[Record], desc: RecordId) -> Result<Vec<RecordId>, RecordId> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut next = Some(desc);
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err(id);
        }
        chain.push(id);
        next = match records.get(id.0) {
            Some(Record::ClassDesc(desc)) => desc.superclass,
            Some(Record::ProxyClassDesc(desc)) => desc.superclass,
            _ => return Err(id),
        };
    }
    chain.reverse();
    Ok(chain)
}
