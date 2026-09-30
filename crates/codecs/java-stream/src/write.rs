//! Graph to bytes.

use crate::error::WriteError;
use crate::model::{
    Annotation, Array, BlockData, ClassData, ClassDesc, ClassRecord, Content, Elements,
    EnumConstant, FieldType, FieldValue, JavaString, Object, ProxyClassDesc, Record, RecordId,
    Stream, Value,
};
use crate::mutf8::Mutf8;
use crate::protocol::{
    BASE_WIRE_HANDLE, STREAM_MAGIC, STREAM_VERSION, TC_ARRAY, TC_BLOCKDATA, TC_BLOCKDATALONG,
    TC_CLASS, TC_CLASSDESC, TC_ENDBLOCKDATA, TC_ENUM, TC_EXCEPTION, TC_LONGSTRING, TC_NULL,
    TC_OBJECT, TC_PROXYCLASSDESC, TC_REFERENCE, TC_RESET, TC_STRING,
};
use crate::read::Limits;

impl Stream {
    /// Writes the stream with the default [`Limits`].
    ///
    /// A stream [`crate::read`] returned writes back to the bytes it was
    /// read from. Handles are assigned in grammar order as the graph is
    /// written: a record is defined at the first position that refers to
    /// it and referenced with `TC_REFERENCE` at every later one.
    ///
    /// # Errors
    ///
    /// A [`WriteError`] when the graph is inconsistent: a dangling or
    /// mistyped record id, class data that does not match its descriptors,
    /// a reference across a reset, or a length its field cannot hold.
    pub fn to_bytes(&self) -> Result<Vec<u8>, WriteError> {
        self.to_bytes_with(&Limits::default())
    }

    /// Writes the stream, refusing graphs nested deeper than
    /// [`Limits::max_depth`]; the other limits concern reading only.
    ///
    /// # Errors
    ///
    /// As [`Stream::to_bytes`].
    pub fn to_bytes_with(&self, limits: &Limits) -> Result<Vec<u8>, WriteError> {
        let mut writer = Writer {
            stream: self,
            out: Vec::new(),
            slots: vec![Slot::Unwritten; self.records.len()],
            epoch: 0,
            next: 0,
            depth: 0,
            max_depth: limits.max_depth,
        };
        writer.stream()?;
        Ok(writer.out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Unwritten,
    /// Written, and still being written (a descriptor's annotation or
    /// superclass).
    Open {
        epoch: u32,
        handle: u32,
    },
    Written {
        epoch: u32,
        handle: u32,
    },
}

struct Writer<'a> {
    stream: &'a Stream,
    out: Vec<u8>,
    slots: Vec<Slot>,
    /// Incremented by every reset, which discards the handle table.
    epoch: u32,
    next: u32,
    depth: usize,
    max_depth: usize,
}

impl<'a> Writer<'a> {
    fn stream(&mut self) -> Result<(), WriteError> {
        self.out.extend_from_slice(&STREAM_MAGIC.to_be_bytes());
        self.out.extend_from_slice(&STREAM_VERSION.to_be_bytes());
        for content in &self.stream.contents {
            match content {
                Content::Value(value) => self.value(*value)?,
                Content::BlockData(block) => self.block(block)?,
                Content::Reset => {
                    self.out.push(TC_RESET);
                    self.reset();
                }
                Content::Exception(exception) => {
                    self.out.extend_from_slice(&exception.aborted);
                    self.out.push(TC_EXCEPTION);
                    self.reset();
                    self.value(exception.throwable)?;
                    self.reset();
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.epoch += 1;
        self.next = 0;
    }

    fn record(&self, id: RecordId) -> Result<&'a Record, WriteError> {
        self.stream.record(id).ok_or(WriteError::UnknownRecord(id))
    }

    /// Enters one nesting level; the caller leaves it with `depth -= 1`.
    fn enter(&mut self) -> Result<(), WriteError> {
        if self.depth >= self.max_depth {
            return Err(WriteError::TooDeep);
        }
        self.depth += 1;
        Ok(())
    }

    /// Writes a reference to `id` if it has a handle, and returns whether
    /// it did; the caller defines the record otherwise.
    fn reference(&mut self, id: RecordId) -> Result<bool, WriteError> {
        match self.slots[id.0] {
            Slot::Unwritten => Ok(false),
            Slot::Open { epoch, handle } | Slot::Written { epoch, handle }
                if epoch == self.epoch =>
            {
                self.out.push(TC_REFERENCE);
                self.out
                    .extend_from_slice(&(BASE_WIRE_HANDLE + handle).to_be_bytes());
                Ok(true)
            }
            _ => Err(WriteError::StaleReference(id)),
        }
    }

    fn assign(&mut self, id: RecordId, open: bool) {
        let (epoch, handle) = (self.epoch, self.next);
        self.next += 1;
        self.slots[id.0] = if open {
            Slot::Open { epoch, handle }
        } else {
            Slot::Written { epoch, handle }
        };
    }

    fn close(&mut self, id: RecordId) {
        if let Slot::Open { epoch, handle } = self.slots[id.0] {
            self.slots[id.0] = Slot::Written { epoch, handle };
        }
    }

    // --- positions -------------------------------------------------------

    fn value(&mut self, value: Value) -> Result<(), WriteError> {
        self.enter()?;
        let result = self.value_at(value);
        self.depth -= 1;
        result
    }

    fn value_at(&mut self, value: Value) -> Result<(), WriteError> {
        let Value::Ref(id) = value else {
            self.out.push(TC_NULL);
            return Ok(());
        };
        let record = self.record(id)?;
        if self.reference(id)? {
            return Ok(());
        }
        self.define(id, record)
    }

    /// A `classDesc`: `None` writes `TC_NULL`. `complete` refuses a
    /// descriptor still being written, which a reader could not use.
    fn desc(&mut self, desc: Option<RecordId>, complete: bool) -> Result<(), WriteError> {
        self.enter()?;
        let result = self.desc_at(desc, complete);
        self.depth -= 1;
        result
    }

    fn desc_at(&mut self, desc: Option<RecordId>, complete: bool) -> Result<(), WriteError> {
        let Some(id) = desc else {
            self.out.push(TC_NULL);
            return Ok(());
        };
        let record = self.record(id)?;
        if !matches!(record, Record::ClassDesc(_) | Record::ProxyClassDesc(_)) {
            return Err(wrong(id, "class descriptor", record));
        }
        if complete && matches!(self.slots[id.0], Slot::Open { .. }) {
            return Err(WriteError::IncompleteClassDesc(id));
        }
        if self.reference(id)? {
            return Ok(());
        }
        self.define(id, record)
    }

    fn string_value(&mut self, value: Value) -> Result<(), WriteError> {
        if let Value::Ref(id) = value {
            let record = self.record(id)?;
            if !matches!(record, Record::String(_)) {
                return Err(wrong(id, "string", record));
            }
        }
        self.value(value)
    }

    fn annotation(&mut self, items: &[Annotation]) -> Result<(), WriteError> {
        for item in items {
            match item {
                Annotation::BlockData(block) => self.block(block)?,
                Annotation::Value(value) => self.value(*value)?,
            }
        }
        self.out.push(TC_ENDBLOCKDATA);
        Ok(())
    }

    // --- records ---------------------------------------------------------

    /// Defines a record at its first position. One function per kind, so
    /// a nesting level only holds the frame of the kind it writes.
    fn define(&mut self, id: RecordId, record: &'a Record) -> Result<(), WriteError> {
        match record {
            Record::ClassDesc(desc) => self.class_desc(id, desc),
            Record::ProxyClassDesc(desc) => self.proxy_class_desc(id, desc),
            Record::Class(class) => self.class(id, class),
            Record::Object(object) => self.object(id, object),
            Record::Array(array) => self.array(id, array),
            Record::String(string) => self.string(id, string),
            Record::Enum(constant) => self.enum_constant(id, constant),
        }
    }

    /// A descriptor stays open until its superclass is written, and a
    /// superclass position refuses an open descriptor, so a cyclic chain is
    /// refused as `IncompleteClassDesc` without walking it.
    fn class_desc(&mut self, id: RecordId, desc: &'a ClassDesc) -> Result<(), WriteError> {
        self.out.push(TC_CLASSDESC);
        self.utf(&desc.name, "class name")?;
        self.out
            .extend_from_slice(&desc.serial_version_uid.to_be_bytes());
        self.assign(id, true);
        self.out.push(desc.flags);
        let count = i16::try_from(desc.fields.len()).map_err(|_| WriteError::TooLong {
            what: "field list",
            length: desc.fields.len(),
        })?;
        self.out.extend_from_slice(&count.to_be_bytes());
        for field in &desc.fields {
            self.out.push(field.ty.code());
            self.utf(&field.name, "field name")?;
            if let FieldType::Array(ty) | FieldType::Object(ty) = field.ty {
                self.string_value(ty)?;
            }
        }
        self.annotation(&desc.annotation)?;
        self.desc(desc.superclass, true)?;
        self.close(id);
        Ok(())
    }

    fn proxy_class_desc(
        &mut self,
        id: RecordId,
        desc: &'a ProxyClassDesc,
    ) -> Result<(), WriteError> {
        self.out.push(TC_PROXYCLASSDESC);
        self.assign(id, true);
        let count = i32::try_from(desc.interfaces.len())
            .ok()
            .filter(|count| *count <= 0xFFFF)
            .ok_or(WriteError::TooLong {
                what: "interface list",
                length: desc.interfaces.len(),
            })?;
        self.out.extend_from_slice(&count.to_be_bytes());
        for name in &desc.interfaces {
            self.utf(name, "interface name")?;
        }
        self.annotation(&desc.annotation)?;
        self.desc(desc.superclass, true)?;
        self.close(id);
        Ok(())
    }

    fn class(&mut self, id: RecordId, class: &'a ClassRecord) -> Result<(), WriteError> {
        self.out.push(TC_CLASS);
        self.desc(Some(class.desc), false)?;
        self.assign(id, false);
        Ok(())
    }

    fn object(&mut self, id: RecordId, object: &'a Object) -> Result<(), WriteError> {
        self.out.push(TC_OBJECT);
        self.desc(Some(object.desc), true)?;
        self.assign(id, false);
        self.class_data(id, object.desc, &object.data)?;
        Ok(())
    }

    fn array(&mut self, id: RecordId, array: &'a Array) -> Result<(), WriteError> {
        self.out.push(TC_ARRAY);
        self.desc(Some(array.desc), false)?;
        let fits = match self.record(array.desc)? {
            Record::ClassDesc(desc) => match (desc.name.as_bytes(), array.elements.code()) {
                ([b'[', b'[' | b'L', ..], b'L') => true,
                ([b'[', code, ..], elements) => *code == elements && elements != b'L',
                _ => false,
            },
            Record::ProxyClassDesc(_) => false,
            other => return Err(wrong(array.desc, "class descriptor", other)),
        };
        if !fits {
            return Err(WriteError::ArrayMismatch(id));
        }
        self.assign(id, false);
        let length = i32::try_from(array.elements.len()).map_err(|_| WriteError::TooLong {
            what: "array",
            length: array.elements.len(),
        })?;
        self.out.extend_from_slice(&length.to_be_bytes());
        self.elements(&array.elements)?;
        Ok(())
    }

    fn string(&mut self, id: RecordId, string: &'a JavaString) -> Result<(), WriteError> {
        let length = string.value.len();
        if string.long {
            self.out.push(TC_LONGSTRING);
            let long = i64::try_from(length).map_err(|_| WriteError::TooLong {
                what: "string",
                length,
            })?;
            self.out.extend_from_slice(&long.to_be_bytes());
        } else {
            let short = u16::try_from(length).map_err(|_| WriteError::TooLong {
                what: "string",
                length,
            })?;
            self.out.push(TC_STRING);
            self.out.extend_from_slice(&short.to_be_bytes());
        }
        self.assign(id, false);
        self.out.extend_from_slice(string.value.as_bytes());
        Ok(())
    }

    fn enum_constant(
        &mut self,
        id: RecordId,
        constant: &'a EnumConstant,
    ) -> Result<(), WriteError> {
        self.out.push(TC_ENUM);
        self.desc(Some(constant.desc), false)?;
        self.assign(id, false);
        self.string_value(Value::Ref(constant.name))?;
        Ok(())
    }

    fn chain_error(&self, at: RecordId) -> WriteError {
        match self.stream.record(at) {
            None => WriteError::UnknownRecord(at),
            Some(Record::ClassDesc(_) | Record::ProxyClassDesc(_)) => {
                WriteError::CyclicSuperclass(at)
            }
            Some(record) => wrong(at, "class descriptor", record),
        }
    }

    fn class_data(
        &mut self,
        id: RecordId,
        desc: RecordId,
        data: &[ClassData],
    ) -> Result<(), WriteError> {
        let mismatch = |reason| WriteError::ClassDataMismatch { record: id, reason };
        if let Record::ClassDesc(own) = self.record(desc)?
            && own.is_externalizable()
        {
            if !own.has_block_data() {
                return Err(mismatch(
                    "externalizable data without SC_BLOCK_DATA cannot be delimited",
                ));
            }
            let [ClassData::External(items)] = data else {
                return Err(mismatch(
                    "an externalizable object needs exactly one External entry",
                ));
            };
            return self.annotation(items);
        }
        let chain = self
            .stream
            .class_chain(desc)
            .map_err(|at| self.chain_error(at))?;
        if chain.len() != data.len() {
            return Err(mismatch("one class data entry per descriptor of the chain"));
        }
        for (slot, data) in chain.into_iter().zip(data) {
            let ClassData::Serial { values, custom } = data else {
                return Err(mismatch("External data for a serializable class"));
            };
            let (fields, write_method) = match self.record(slot)? {
                Record::ClassDesc(desc) => (desc.fields.as_slice(), desc.has_write_method()),
                _ => (&[][..], false),
            };
            if values.len() != fields.len()
                || !values
                    .iter()
                    .zip(fields)
                    .all(|(value, field)| value.fits(field.ty))
            {
                return Err(mismatch(
                    "field values do not match the descriptor's fields",
                ));
            }
            if custom.is_some() != write_method {
                return Err(mismatch(
                    "custom data present exactly when SC_WRITE_METHOD is set",
                ));
            }
            for value in values {
                self.field_value(value)?;
            }
            if let Some(items) = custom {
                self.annotation(items)?;
            }
        }
        Ok(())
    }

    fn field_value(&mut self, value: &FieldValue) -> Result<(), WriteError> {
        match *value {
            FieldValue::Byte(v) => self.out.extend_from_slice(&v.to_be_bytes()),
            FieldValue::Char(v) => self.out.extend_from_slice(&v.to_be_bytes()),
            FieldValue::Double(v) => self.out.extend_from_slice(&v.to_bits().to_be_bytes()),
            FieldValue::Float(v) => self.out.extend_from_slice(&v.to_bits().to_be_bytes()),
            FieldValue::Int(v) => self.out.extend_from_slice(&v.to_be_bytes()),
            FieldValue::Long(v) => self.out.extend_from_slice(&v.to_be_bytes()),
            FieldValue::Short(v) => self.out.extend_from_slice(&v.to_be_bytes()),
            FieldValue::Boolean(v) => self.out.push(u8::from(v)),
            FieldValue::Object(v) => self.value(v)?,
        }
        Ok(())
    }

    fn elements(&mut self, elements: &Elements) -> Result<(), WriteError> {
        match elements {
            Elements::Byte(v) => self.out.extend_from_slice(v),
            Elements::Boolean(v) => self.out.extend(v.iter().map(|b| u8::from(*b))),
            Elements::Char(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_be_bytes())),
            Elements::Short(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_be_bytes())),
            Elements::Int(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_be_bytes())),
            Elements::Float(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_bits().to_be_bytes())),
            Elements::Long(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_be_bytes())),
            Elements::Double(v) => v
                .iter()
                .for_each(|e| self.out.extend_from_slice(&e.to_bits().to_be_bytes())),
            Elements::Object(v) => {
                for value in v {
                    self.value(*value)?;
                }
            }
        }
        Ok(())
    }

    fn block(&mut self, block: &BlockData) -> Result<(), WriteError> {
        let length = block.bytes.len();
        if block.long {
            let long = i32::try_from(length).map_err(|_| WriteError::TooLong {
                what: "block data",
                length,
            })?;
            self.out.push(TC_BLOCKDATALONG);
            self.out.extend_from_slice(&long.to_be_bytes());
        } else {
            let short = u8::try_from(length).map_err(|_| WriteError::TooLong {
                what: "block data",
                length,
            })?;
            self.out.push(TC_BLOCKDATA);
            self.out.push(short);
        }
        self.out.extend_from_slice(&block.bytes);
        Ok(())
    }

    fn utf(&mut self, text: &Mutf8, what: &'static str) -> Result<(), WriteError> {
        let length = u16::try_from(text.len()).map_err(|_| WriteError::TooLong {
            what,
            length: text.len(),
        })?;
        self.out.extend_from_slice(&length.to_be_bytes());
        self.out.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

fn wrong(record: RecordId, expected: &'static str, found: &Record) -> WriteError {
    WriteError::WrongRecord {
        record,
        expected,
        found: found.kind(),
    }
}
