//! A codec for Java object serialization streams.
//!
//! Implements the stream protocol of the public *Java Object Serialization
//! Specification* (chapter 6, "Object Serialization Stream Protocol"), in
//! both directions:
//!
//! - [`read`] turns the bytes of a stream into a [`Stream`]: its top-level
//!   contents and a graph of [`Record`]s, one per handle the stream assigns.
//! - [`Stream::to_bytes`] writes a graph back. A stream that was read writes
//!   back byte for byte, handle assignment order included.
//! - [`Stream::new`], [`Stream::add`] and [`Stream::push`] build new graphs;
//!   the writer allocates handles as the grammar assigns them.
//!
//! The codec knows the protocol, not any class. Field values are decoded
//! by the types their class descriptors declare; what a class wrote in a
//! custom `writeObject` or `writeExternal` stays an ordered sequence of
//! opaque block data and objects ([`Annotation`]), because only that class
//! knows its layout. Bindings to particular classes belong to the
//! applications that own them.
//!
//! # Untrusted input
//!
//! Reading never panics and never trusts a length: every array, string and
//! block is checked against the remaining input before it is allocated,
//! and [`Limits`] bounds nesting depth, handles, array and string lengths
//! and the total memory of the graph. Malformed input is refused with a
//! typed [`ReadError`].
//!
//! # Coverage
//!
//! Every `TC_*` record of the grammar is read and written, and every class
//! descriptor flag honored. One construct is refused rather than read:
//! externalizable data written with protocol version 1 (no
//! `SC_BLOCK_DATA`), which only the class's own `readExternal` can
//! delimit ([`ReadError::ExternalWithoutBlockData`]). The grammar assumes a
//! custom `writeObject` writes its default fields first, as the
//! specification requires; a class that writes custom data before them
//! cannot be read by structure alone.
//!
//! ```
//! use axioval_java_stream::{ClassData, ClassDesc, FieldType, FieldValue, Stream, SC_SERIALIZABLE, read};
//!
//! let mut stream = Stream::new();
//! let desc = stream.add_class_desc(ClassDesc::new("Point", 1, SC_SERIALIZABLE).with_field("x", FieldType::Int));
//! let point = stream.add_object(desc, vec![ClassData::Serial { values: vec![FieldValue::Int(3)], custom: None }]);
//! stream.push_value(point);
//! stream.push_value(point); // written as TC_REFERENCE to the first
//!
//! let bytes = stream.to_bytes()?;
//! let back = read(&bytes)?;
//! assert_eq!(back, stream);
//! assert_eq!(back.to_bytes()?, bytes);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod model;
mod mutf8;
mod read;
mod write;

pub use error::{Limit, ReadError, WriteError};
pub use model::{
    Annotation, Array, BlockData, ClassData, ClassDesc, ClassRecord, Content, Elements,
    EnumConstant, Exception, FieldDesc, FieldType, FieldValue, JavaString, Object, ProxyClassDesc,
    Record, RecordId, Stream, Value,
};
pub use mutf8::{LoneSurrogate, Mutf8, Mutf8Error};
pub use protocol::*;
pub use read::{Limits, read, read_with};

/// The terminal symbols and constants of the grammar (specification §6.4.2).
mod protocol {
    /// First two bytes of every stream.
    pub const STREAM_MAGIC: u16 = 0xACED;
    /// The stream version this codec reads and writes.
    pub const STREAM_VERSION: u16 = 5;
    /// A null reference.
    pub const TC_NULL: u8 = 0x70;
    /// A reference to an earlier handle.
    pub const TC_REFERENCE: u8 = 0x71;
    /// A class descriptor.
    pub const TC_CLASSDESC: u8 = 0x72;
    /// An object.
    pub const TC_OBJECT: u8 = 0x73;
    /// A string of at most 65535 bytes.
    pub const TC_STRING: u8 = 0x74;
    /// An array.
    pub const TC_ARRAY: u8 = 0x75;
    /// A `java.lang.Class`.
    pub const TC_CLASS: u8 = 0x76;
    /// Block data of at most 255 bytes.
    pub const TC_BLOCKDATA: u8 = 0x77;
    /// The end of an annotation or of custom data.
    pub const TC_ENDBLOCKDATA: u8 = 0x78;
    /// A reset of the handle table.
    pub const TC_RESET: u8 = 0x79;
    /// Block data with a four-byte length.
    pub const TC_BLOCKDATALONG: u8 = 0x7A;
    /// An exception that aborted a write.
    pub const TC_EXCEPTION: u8 = 0x7B;
    /// A string with an eight-byte length.
    pub const TC_LONGSTRING: u8 = 0x7C;
    /// A proxy class descriptor.
    pub const TC_PROXYCLASSDESC: u8 = 0x7D;
    /// An enum constant.
    pub const TC_ENUM: u8 = 0x7E;
    /// The first handle assigned after the start or a reset.
    pub const BASE_WIRE_HANDLE: u32 = 0x7E_0000;
    /// The class has a `writeObject` method that may write custom data.
    pub const SC_WRITE_METHOD: u8 = 0x01;
    /// The class is `Serializable`.
    pub const SC_SERIALIZABLE: u8 = 0x02;
    /// The class is `Externalizable`.
    pub const SC_EXTERNALIZABLE: u8 = 0x04;
    /// Externalizable data is written as block data (protocol version 2).
    pub const SC_BLOCK_DATA: u8 = 0x08;
    /// The class is an enum type.
    pub const SC_ENUM: u8 = 0x10;
}
