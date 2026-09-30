//! Bytes to graph.

use std::mem::size_of;

use crate::error::{Limit, ReadError};
use crate::model::{
    Annotation, Array, BlockData, ClassData, ClassDesc, ClassRecord, Content, Elements,
    EnumConstant, Exception, FieldDesc, FieldType, FieldValue, JavaString, Object, ProxyClassDesc,
    Record, RecordId, Stream, Value, class_chain,
};
use crate::mutf8::{Mutf8, validate};
use crate::protocol::{
    BASE_WIRE_HANDLE, STREAM_MAGIC, STREAM_VERSION, TC_ARRAY, TC_BLOCKDATA, TC_BLOCKDATALONG,
    TC_CLASS, TC_CLASSDESC, TC_ENDBLOCKDATA, TC_ENUM, TC_EXCEPTION, TC_LONGSTRING, TC_NULL,
    TC_OBJECT, TC_PROXYCLASSDESC, TC_REFERENCE, TC_RESET, TC_STRING,
};

/// Hard limits a reader enforces, so hostile input cannot exhaust memory
/// or the stack. Every limit is checked before the memory it guards is
/// allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How deeply objects, descriptors and type strings may nest. Each
    /// level costs stack in the reader and the writer: the default fits a
    /// 2 MiB thread stack with room to spare even unoptimized, so raise it
    /// only on a thread with a larger stack.
    pub max_depth: usize,
    /// How many handles one handle table (between resets) may hold.
    pub max_handles: usize,
    /// The largest array length.
    pub max_array_length: usize,
    /// The longest string, in bytes of modified UTF-8.
    pub max_string_length: usize,
    /// An estimate of the bytes the graph may occupy in memory, summed over
    /// every record, value, string, array and block of data.
    pub max_allocation: usize,
}

impl Default for Limits {
    /// Limits that admit any stream a well-behaved application writes: 256
    /// levels, a million handles, 16 Mi elements or bytes per array or
    /// string, and 256 MiB of graph.
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_handles: 1 << 20,
            max_array_length: 1 << 24,
            max_string_length: 1 << 24,
            max_allocation: 256 << 20,
        }
    }
}

/// Reads a whole stream with the default [`Limits`].
///
/// The stream must end exactly after its last item: the grammar reads
/// contents up to the end of input, so a trailing byte is an error.
///
/// # Errors
///
/// A [`ReadError`] naming the first malformed item or exceeded limit.
pub fn read(bytes: &[u8]) -> Result<Stream, ReadError> {
    read_with(bytes, &Limits::default())
}

/// Reads a whole stream with the given limits.
///
/// # Errors
///
/// A [`ReadError`] naming the first malformed item or exceeded limit.
pub fn read_with(bytes: &[u8], limits: &Limits) -> Result<Stream, ReadError> {
    Reader {
        input: bytes,
        pos: 0,
        limits: *limits,
        records: Vec::new(),
        complete: Vec::new(),
        handles: Vec::new(),
        depth: 0,
        allocated: 0,
        in_exception: false,
    }
    .stream()
}

/// Why a nested read stopped: an error, or `TC_EXCEPTION` aborting the
/// enclosing top-level object.
enum Flow {
    Error(Box<ReadError>),
    Abort,
}

impl From<ReadError> for Flow {
    fn from(error: ReadError) -> Self {
        Self::Error(Box::new(error))
    }
}

type Flowing<T> = Result<T, Flow>;

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
    limits: Limits,
    records: Vec<Record>,
    /// Whether each record has been read to its end; descriptors are not
    /// while their annotation and superclass are read.
    complete: Vec<bool>,
    /// The current handle table: wire handle minus `baseWireHandle` to record.
    handles: Vec<RecordId>,
    depth: usize,
    allocated: usize,
    in_exception: bool,
}

impl<'a> Reader<'a> {
    fn stream(mut self) -> Result<Stream, ReadError> {
        let magic = self.u16()?;
        if magic != STREAM_MAGIC {
            return Err(ReadError::BadMagic { found: magic });
        }
        let version = self.u16()?;
        if version != STREAM_VERSION {
            return Err(ReadError::UnsupportedVersion { found: version });
        }
        let mut contents = Vec::new();
        while self.pos < self.input.len() {
            let start = self.pos;
            let defined = self.records.len();
            self.charge(size_of::<Content>(), start)?;
            let item = match self.input[start] {
                TC_RESET => {
                    self.pos += 1;
                    self.handles.clear();
                    Content::Reset
                }
                TC_BLOCKDATA | TC_BLOCKDATALONG => Content::BlockData(self.block()?),
                TC_EXCEPTION => Content::Exception(self.exception(Vec::new())?),
                _ => match self.value() {
                    Ok(value) => Content::Value(value),
                    Err(Flow::Error(error)) => return Err(*error),
                    Err(Flow::Abort) => {
                        // What the writer had written of the aborted object is
                        // discarded with its handles; keep its bytes only.
                        let aborted = self.input[start..self.pos].to_vec();
                        self.records.truncate(defined);
                        self.complete.truncate(defined);
                        Content::Exception(self.exception(aborted)?)
                    }
                },
            };
            contents.push(item);
        }
        Ok(Stream {
            contents,
            records: self.records,
        })
    }

    /// `TC_EXCEPTION reset (Throwable)object reset`, at `self.pos`.
    fn exception(&mut self, aborted: Vec<u8>) -> Result<Exception, ReadError> {
        self.pos += 1;
        self.handles.clear();
        self.in_exception = true;
        let throwable = match self.value() {
            Ok(value) => value,
            Err(Flow::Error(error)) => return Err(*error),
            // `value` refuses a nested TC_EXCEPTION while in_exception.
            Err(Flow::Abort) => return Err(ReadError::NestedException { offset: self.pos }),
        };
        self.in_exception = false;
        self.handles.clear();
        Ok(Exception { aborted, throwable })
    }

    // --- positions -------------------------------------------------------

    /// Enters one nesting level; the caller leaves it with `depth -= 1`.
    /// (No closure: every frame per level costs stack.)
    fn enter(&mut self) -> Result<(), ReadError> {
        if self.depth >= self.limits.max_depth {
            return Err(limit(Limit::Depth, self.pos));
        }
        self.depth += 1;
        Ok(())
    }

    /// An `object`: any record, a reference or null.
    fn value(&mut self) -> Flowing<Value> {
        self.enter()?;
        let result = self.value_at();
        self.depth -= 1;
        result
    }

    fn value_at(&mut self) -> Flowing<Value> {
        let offset = self.pos;
        let code = self.peek()?;
        let id = match code {
            TC_NULL => {
                self.pos += 1;
                return Ok(Value::Null);
            }
            TC_REFERENCE => self.reference(None)?,
            TC_CLASSDESC => self.class_desc()?,
            TC_PROXYCLASSDESC => self.proxy_class_desc()?,
            TC_OBJECT => self.object()?,
            TC_CLASS => self.class()?,
            TC_ARRAY => self.array()?,
            TC_STRING | TC_LONGSTRING => self.string()?,
            TC_ENUM => self.enum_constant()?,
            TC_EXCEPTION if self.in_exception => {
                return Err(ReadError::NestedException { offset }.into());
            }
            TC_EXCEPTION => return Err(Flow::Abort),
            TC_RESET => return Err(ReadError::NestedReset { offset }.into()),
            code => return Err(unexpected(offset, code, "an object").into()),
        };
        Ok(Value::Ref(id))
    }

    /// A `classDesc`. `complete` requires a referenced descriptor to have
    /// been read to its end, where its superclass chain is needed.
    fn desc(&mut self, nullable: bool, complete: bool) -> Flowing<Option<RecordId>> {
        self.enter()?;
        let result = self.desc_at(nullable, complete);
        self.depth -= 1;
        result
    }

    fn desc_at(&mut self, nullable: bool, complete: bool) -> Flowing<Option<RecordId>> {
        let offset = self.pos;
        match self.peek()? {
            TC_NULL if nullable => {
                self.pos += 1;
                Ok(None)
            }
            TC_NULL => Err(ReadError::UnexpectedNull {
                offset,
                expected: "a class descriptor",
            }
            .into()),
            TC_REFERENCE => {
                let id = self.reference(Some("class descriptor"))?;
                if complete && !self.complete[id.0] {
                    return Err(ReadError::IncompleteClassDesc { offset }.into());
                }
                Ok(Some(id))
            }
            TC_CLASSDESC => Ok(Some(self.class_desc()?)),
            TC_PROXYCLASSDESC => Ok(Some(self.proxy_class_desc()?)),
            code => Err(unexpected(offset, code, "a class descriptor").into()),
        }
    }

    /// A `(String)object`: a field's type string or an enum constant name.
    fn string_value(&mut self, nullable: bool) -> Flowing<Value> {
        self.enter()?;
        let result = self.string_value_at(nullable);
        self.depth -= 1;
        result
    }

    fn string_value_at(&mut self, nullable: bool) -> Flowing<Value> {
        let offset = self.pos;
        match self.peek()? {
            TC_NULL if nullable => {
                self.pos += 1;
                Ok(Value::Null)
            }
            TC_NULL => Err(ReadError::UnexpectedNull {
                offset,
                expected: "a string",
            }
            .into()),
            TC_REFERENCE => Ok(Value::Ref(self.reference(Some("string"))?)),
            TC_STRING | TC_LONGSTRING => Ok(Value::Ref(self.string()?)),
            code => Err(unexpected(offset, code, "a string").into()),
        }
    }

    /// `contents endBlockData` of an annotation or custom data.
    fn annotation(&mut self) -> Flowing<Vec<Annotation>> {
        let mut items = Vec::new();
        loop {
            let offset = self.pos;
            let code = self.peek()?;
            if code == TC_ENDBLOCKDATA {
                self.pos += 1;
                return Ok(items);
            }
            self.charge(size_of::<Annotation>(), offset)?;
            items.push(match code {
                TC_BLOCKDATA | TC_BLOCKDATALONG => Annotation::BlockData(self.block()?),
                _ => Annotation::Value(self.value()?),
            });
        }
    }

    // --- records ---------------------------------------------------------

    fn reference(&mut self, expected: Option<&'static str>) -> Result<RecordId, ReadError> {
        let offset = self.pos;
        self.pos += 1;
        let handle = self.u32()?;
        let id = handle
            .checked_sub(BASE_WIRE_HANDLE)
            .and_then(|index| self.handles.get(index as usize).copied())
            .ok_or(ReadError::UnknownHandle { offset, handle })?;
        if let Some(expected) = expected {
            let found = self.records[id.0].kind();
            let fits = match expected {
                "class descriptor" => matches!(
                    self.records[id.0],
                    Record::ClassDesc(_) | Record::ProxyClassDesc(_)
                ),
                _ => found == expected,
            };
            if !fits {
                return Err(ReadError::WrongReference {
                    offset,
                    expected,
                    found,
                });
            }
        }
        Ok(id)
    }

    fn class_desc(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let name = self.utf()?;
        let serial_version_uid = self.i64()?;
        let mut desc = ClassDesc {
            name,
            serial_version_uid,
            flags: 0,
            fields: Vec::new(),
            annotation: Vec::new(),
            superclass: None,
        };
        let id = self.assign(Record::ClassDesc(desc.clone()), false, offset)?;
        desc.flags = self.u8()?;
        let count_at = self.pos;
        let count = self.i16()?;
        if count < 0 {
            return Err(ReadError::NegativeLength {
                offset: count_at,
                length: count.into(),
            }
            .into());
        }
        for _ in 0..count {
            let at = self.pos;
            self.charge(size_of::<FieldDesc>(), at)?;
            let code = self.u8()?;
            let name = self.utf()?;
            let ty = match code {
                b'[' => FieldType::Array(self.string_value(true)?),
                b'L' => FieldType::Object(self.string_value(true)?),
                code => FieldType::primitive(code)
                    .ok_or(ReadError::InvalidFieldType { offset: at, code })?,
            };
            desc.fields.push(FieldDesc { name, ty });
        }
        if desc.is_serializable() && desc.is_externalizable() {
            return Err(ReadError::ConflictingFlags {
                offset,
                flags: desc.flags,
            }
            .into());
        }
        desc.annotation = self.annotation()?;
        desc.superclass = self.desc(true, true)?;
        self.records[id.0] = Record::ClassDesc(desc);
        self.complete[id.0] = true;
        Ok(id)
    }

    fn proxy_class_desc(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let mut desc = ProxyClassDesc {
            interfaces: Vec::new(),
            annotation: Vec::new(),
            superclass: None,
        };
        let id = self.assign(Record::ProxyClassDesc(desc.clone()), false, offset)?;
        let count_at = self.pos;
        let count = self.i32()?;
        if count < 0 {
            return Err(ReadError::NegativeLength {
                offset: count_at,
                length: count.into(),
            }
            .into());
        }
        if count > 0xFFFF {
            return Err(ReadError::TooManyInterfaces {
                offset: count_at,
                count,
            }
            .into());
        }
        for _ in 0..count {
            self.charge(size_of::<Mutf8>(), self.pos)?;
            let name = self.utf()?;
            desc.interfaces.push(name);
        }
        desc.annotation = self.annotation()?;
        desc.superclass = self.desc(true, true)?;
        self.records[id.0] = Record::ProxyClassDesc(desc);
        self.complete[id.0] = true;
        Ok(id)
    }

    fn class(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let desc = self.required_desc(false)?;
        Ok(self.assign(Record::Class(ClassRecord { desc }), true, offset)?)
    }

    fn object(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let desc = self.required_desc(true)?;
        let id = self.assign(
            Record::Object(Object {
                desc,
                data: Vec::new(),
            }),
            true,
            offset,
        )?;
        let data = self.class_data(desc, offset)?;
        self.records[id.0] = Record::Object(Object { desc, data });
        Ok(id)
    }

    fn class_data(&mut self, desc: RecordId, offset: usize) -> Flowing<Vec<ClassData>> {
        if let Record::ClassDesc(own) = &self.records[desc.0]
            && own.is_externalizable()
        {
            if !own.has_block_data() {
                return Err(ReadError::ExternalWithoutBlockData { offset }.into());
            }
            self.charge(size_of::<ClassData>(), offset)?;
            return Ok(vec![ClassData::External(self.annotation()?)]);
        }
        // A complete descriptor has an acyclic chain of complete descriptors,
        // so this cannot fail; it fails closed all the same.
        let chain = class_chain(&self.records, desc)
            .map_err(|_| ReadError::IncompleteClassDesc { offset })?;
        self.charge(chain.len().saturating_mul(size_of::<ClassData>()), offset)?;
        let mut data = Vec::with_capacity(chain.len());
        for slot in chain {
            let (count, custom) = match &self.records[slot.0] {
                Record::ClassDesc(desc) => (desc.fields.len(), desc.has_write_method()),
                _ => (0, false),
            };
            self.charge(count.saturating_mul(size_of::<FieldValue>()), self.pos)?;
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                let ty = match &self.records[slot.0] {
                    Record::ClassDesc(desc) => desc.fields[index].ty,
                    _ => FieldType::Byte,
                };
                values.push(self.field_value(ty)?);
            }
            let custom = if custom {
                Some(self.annotation()?)
            } else {
                None
            };
            data.push(ClassData::Serial { values, custom });
        }
        Ok(data)
    }

    fn field_value(&mut self, ty: FieldType) -> Flowing<FieldValue> {
        Ok(match ty {
            FieldType::Byte => FieldValue::Byte(i8::from_be_bytes([self.u8()?])),
            FieldType::Char => FieldValue::Char(self.u16()?),
            FieldType::Double => FieldValue::Double(f64::from_bits(self.u64()?)),
            FieldType::Float => FieldValue::Float(f32::from_bits(self.u32()?)),
            FieldType::Int => FieldValue::Int(self.i32()?),
            FieldType::Long => FieldValue::Long(self.i64()?),
            FieldType::Short => FieldValue::Short(self.i16()?),
            FieldType::Boolean => FieldValue::Boolean(self.boolean()?),
            FieldType::Array(_) | FieldType::Object(_) => FieldValue::Object(self.value()?),
        })
    }

    fn array(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let desc = self.required_desc(false)?;
        let code = match &self.records[desc.0] {
            Record::ClassDesc(desc) => match desc.name.as_bytes() {
                [b'[', code, ..] => *code,
                _ => 0,
            },
            _ => 0,
        };
        let width = match code {
            b'B' | b'Z' | b'[' | b'L' => 1,
            b'C' | b'S' => 2,
            b'I' | b'F' => 4,
            b'J' | b'D' => 8,
            _ => return Err(ReadError::InvalidArrayClass { offset }.into()),
        };
        let id = self.assign(
            Record::Array(Array {
                desc,
                elements: Elements::Object(Vec::new()),
            }),
            true,
            offset,
        )?;
        let length_at = self.pos;
        let length = self.i32()?;
        let length = usize::try_from(length).map_err(|_| ReadError::NegativeLength {
            offset: length_at,
            length: length.into(),
        })?;
        if length > self.limits.max_array_length {
            return Err(limit(Limit::ArrayLength, length_at).into());
        }
        // Every element takes at least `width` bytes: refuse a length the
        // input cannot hold before allocating for it.
        if length.saturating_mul(width) > self.input.len() - self.pos {
            return Err(ReadError::Truncated {
                offset: self.input.len(),
            }
            .into());
        }
        let memory = match code {
            b'[' | b'L' => size_of::<Value>(),
            _ => width,
        };
        self.charge(length.saturating_mul(memory), length_at)?;
        let elements = match code {
            b'B' => Elements::Byte(self.take(length)?.to_vec()),
            b'Z' => Elements::Boolean(
                (0..length)
                    .map(|_| self.boolean())
                    .collect::<Result<_, _>>()?,
            ),
            b'C' => Elements::Char((0..length).map(|_| self.u16()).collect::<Result<_, _>>()?),
            b'S' => Elements::Short((0..length).map(|_| self.i16()).collect::<Result<_, _>>()?),
            b'I' => Elements::Int((0..length).map(|_| self.i32()).collect::<Result<_, _>>()?),
            b'F' => Elements::Float(
                (0..length)
                    .map(|_| self.u32().map(f32::from_bits))
                    .collect::<Result<_, _>>()?,
            ),
            b'J' => Elements::Long((0..length).map(|_| self.i64()).collect::<Result<_, _>>()?),
            b'D' => Elements::Double(
                (0..length)
                    .map(|_| self.u64().map(f64::from_bits))
                    .collect::<Result<_, _>>()?,
            ),
            _ => {
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(self.value()?);
                }
                Elements::Object(values)
            }
        };
        self.records[id.0] = Record::Array(Array { desc, elements });
        Ok(id)
    }

    fn string(&mut self) -> Result<RecordId, ReadError> {
        let offset = self.pos;
        let long = self.u8()? == TC_LONGSTRING;
        let length_at = self.pos;
        let length = if long {
            let length = self.i64()?;
            usize::try_from(length).map_err(|_| ReadError::NegativeLength {
                offset: length_at,
                length,
            })?
        } else {
            usize::from(self.u16()?)
        };
        if length > self.limits.max_string_length {
            return Err(limit(Limit::StringLength, length_at));
        }
        let bytes = self.take(length)?;
        validate(bytes).map_err(|_| ReadError::InvalidUtf8 { offset })?;
        let value = Mutf8::from_validated(bytes.to_vec());
        self.charge(length, offset)?;
        self.assign(Record::String(JavaString { value, long }), true, offset)
    }

    fn enum_constant(&mut self) -> Flowing<RecordId> {
        let offset = self.pos;
        self.pos += 1;
        let desc = self.required_desc(false)?;
        let id = self.assign(
            Record::Enum(EnumConstant {
                desc,
                name: RecordId(usize::MAX),
            }),
            true,
            offset,
        )?;
        let Value::Ref(name) = self.string_value(false)? else {
            return Err(ReadError::UnexpectedNull {
                offset,
                expected: "a string",
            }
            .into());
        };
        self.records[id.0] = Record::Enum(EnumConstant { desc, name });
        Ok(id)
    }

    fn required_desc(&mut self, complete: bool) -> Flowing<RecordId> {
        let offset = self.pos;
        self.desc(false, complete)?.ok_or_else(|| {
            ReadError::UnexpectedNull {
                offset,
                expected: "a class descriptor",
            }
            .into()
        })
    }

    fn block(&mut self) -> Result<BlockData, ReadError> {
        let offset = self.pos;
        let long = self.u8()? == TC_BLOCKDATALONG;
        let length_at = self.pos;
        let length = if long {
            let length = self.i32()?;
            usize::try_from(length).map_err(|_| ReadError::NegativeLength {
                offset: length_at,
                length: length.into(),
            })?
        } else {
            usize::from(self.u8()?)
        };
        let bytes = self.take(length)?.to_vec();
        self.charge(length, offset)?;
        Ok(BlockData { bytes, long })
    }

    /// Assigns the next handle to a new record.
    fn assign(
        &mut self,
        record: Record,
        complete: bool,
        offset: usize,
    ) -> Result<RecordId, ReadError> {
        if self.handles.len() >= self.limits.max_handles {
            return Err(limit(Limit::Handles, offset));
        }
        self.charge(size_of::<Record>() + size_of::<RecordId>() + 1, offset)?;
        let id = RecordId(self.records.len());
        self.records.push(record);
        self.complete.push(complete);
        self.handles.push(id);
        Ok(id)
    }

    // --- primitives ------------------------------------------------------

    fn charge(&mut self, bytes: usize, offset: usize) -> Result<(), ReadError> {
        self.allocated = self.allocated.saturating_add(bytes);
        if self.allocated > self.limits.max_allocation {
            return Err(limit(Limit::Allocation, offset));
        }
        Ok(())
    }

    fn peek(&self) -> Result<u8, ReadError> {
        self.input
            .get(self.pos)
            .copied()
            .ok_or(ReadError::Truncated { offset: self.pos })
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ReadError> {
        let end = self
            .pos
            .checked_add(length)
            .filter(|end| *end <= self.input.len());
        let Some(end) = end else {
            return Err(ReadError::Truncated {
                offset: self.input.len(),
            });
        };
        let bytes = &self.input[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn array_of<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.array_of::<1>()?[0])
    }

    fn boolean(&mut self) -> Result<bool, ReadError> {
        let offset = self.pos;
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ReadError::InvalidBoolean { offset, value }),
        }
    }

    fn u16(&mut self) -> Result<u16, ReadError> {
        self.array_of().map(u16::from_be_bytes)
    }

    fn i16(&mut self) -> Result<i16, ReadError> {
        self.array_of().map(i16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, ReadError> {
        self.array_of().map(u32::from_be_bytes)
    }

    fn i32(&mut self) -> Result<i32, ReadError> {
        self.array_of().map(i32::from_be_bytes)
    }

    fn u64(&mut self) -> Result<u64, ReadError> {
        self.array_of().map(u64::from_be_bytes)
    }

    fn i64(&mut self) -> Result<i64, ReadError> {
        self.array_of().map(i64::from_be_bytes)
    }

    /// A `(utf)`: two-byte length, then modified UTF-8.
    fn utf(&mut self) -> Result<Mutf8, ReadError> {
        let offset = self.pos;
        let length = usize::from(self.u16()?);
        if length > self.limits.max_string_length {
            return Err(limit(Limit::StringLength, offset));
        }
        let bytes = self.take(length)?;
        validate(bytes).map_err(|_| ReadError::InvalidUtf8 { offset })?;
        let value = Mutf8::from_validated(bytes.to_vec());
        self.charge(length + size_of::<Mutf8>(), offset)?;
        Ok(value)
    }
}

fn limit(limit: Limit, offset: usize) -> ReadError {
    ReadError::Limit { offset, limit }
}

fn unexpected(offset: usize, code: u8, expected: &'static str) -> ReadError {
    if (TC_NULL..=TC_ENUM).contains(&code) {
        ReadError::Unexpected {
            offset,
            code,
            expected,
        }
    } else {
        ReadError::UnknownTypeCode { offset, code }
    }
}
