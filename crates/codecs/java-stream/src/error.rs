//! Typed read and write errors.

use crate::RecordId;

/// A limit of [`crate::Limits`] a stream exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// [`crate::Limits::max_depth`].
    Depth,
    /// [`crate::Limits::max_handles`].
    Handles,
    /// [`crate::Limits::max_array_length`].
    ArrayLength,
    /// [`crate::Limits::max_string_length`].
    StringLength,
    /// [`crate::Limits::max_allocation`].
    Allocation,
}

/// Why a stream could not be read. `offset` is the byte where the
/// offending item starts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The input ended inside an item.
    #[error("stream truncated at byte {offset}")]
    Truncated {
        /// Where the missing bytes were expected.
        offset: usize,
    },
    /// The stream does not start with `STREAM_MAGIC`.
    #[error("not a Java object serialization stream (magic {found:#06x})")]
    BadMagic {
        /// The first two bytes.
        found: u16,
    },
    /// The stream version is not `STREAM_VERSION` (5).
    #[error("unsupported stream version {found}")]
    UnsupportedVersion {
        /// The version read.
        found: u16,
    },
    /// A byte that is no `TC_*` type code where one was expected.
    #[error("unknown type code {code:#04x} at byte {offset}")]
    UnknownTypeCode {
        /// Where the byte is.
        offset: usize,
        /// The byte.
        code: u8,
    },
    /// A type code the grammar does not allow at this position.
    #[error("type code {code:#04x} at byte {offset} where {expected} was expected")]
    Unexpected {
        /// Where the type code is.
        offset: usize,
        /// The type code.
        code: u8,
        /// What the grammar allows here.
        expected: &'static str,
    },
    /// `TC_REFERENCE` to a handle that was never assigned.
    #[error("reference to unknown handle {handle:#x} at byte {offset}")]
    UnknownHandle {
        /// Where the reference is.
        offset: usize,
        /// The wire handle.
        handle: u32,
    },
    /// `TC_REFERENCE` to a record of the wrong kind for its position.
    #[error("reference at byte {offset} names a {found} where {expected} was expected")]
    WrongReference {
        /// Where the reference is.
        offset: usize,
        /// The kind the position requires.
        expected: &'static str,
        /// The kind the handle names.
        found: &'static str,
    },
    /// `TC_NULL` where the grammar requires an object.
    #[error("null at byte {offset} where {expected} was expected")]
    UnexpectedNull {
        /// Where the null is.
        offset: usize,
        /// What was expected.
        expected: &'static str,
    },
    /// A string that is not modified UTF-8.
    #[error("invalid modified UTF-8 in string at byte {offset}")]
    InvalidUtf8 {
        /// Where the string starts.
        offset: usize,
    },
    /// A negative length or count.
    #[error("negative length {length} at byte {offset}")]
    NegativeLength {
        /// Where the length is.
        offset: usize,
        /// The length read.
        length: i64,
    },
    /// A field type code that is none of `BCDFIJSZ[L`.
    #[error("invalid field type code {code:#04x} at byte {offset}")]
    InvalidFieldType {
        /// Where the code is.
        offset: usize,
        /// The code.
        code: u8,
    },
    /// A boolean byte other than 0 or 1.
    #[error("boolean byte {value:#04x} at byte {offset} is neither 0 nor 1")]
    InvalidBoolean {
        /// Where the byte is.
        offset: usize,
        /// The byte.
        value: u8,
    },
    /// An array whose descriptor names no array class.
    #[error("array at byte {offset} has a descriptor that names no array class")]
    InvalidArrayClass {
        /// Where the array starts.
        offset: usize,
    },
    /// A descriptor that is both serializable and externalizable.
    #[error(
        "class descriptor at byte {offset} is both serializable and externalizable ({flags:#04x})"
    )]
    ConflictingFlags {
        /// Where the descriptor starts.
        offset: usize,
        /// The flags byte.
        flags: u8,
    },
    /// Externalizable data written with protocol version 1, which only the
    /// class's own `readExternal` can delimit.
    #[error(
        "externalizable object at byte {offset} was written without block data (protocol version 1)"
    )]
    ExternalWithoutBlockData {
        /// Where the object starts.
        offset: usize,
    },
    /// A proxy descriptor naming more than 65535 interfaces.
    #[error("proxy class descriptor at byte {offset} names {count} interfaces")]
    TooManyInterfaces {
        /// Where the count is.
        offset: usize,
        /// The count.
        count: i32,
    },
    /// A descriptor used before its superclass has been read: as its own
    /// ancestor, or for an object or array inside its own annotation.
    #[error("class descriptor referenced at byte {offset} is still being read")]
    IncompleteClassDesc {
        /// Where the reference is.
        offset: usize,
    },
    /// `TC_RESET` inside an object.
    #[error("reset inside an object at byte {offset}")]
    NestedReset {
        /// Where the reset is.
        offset: usize,
    },
    /// `TC_EXCEPTION` while reading an exception.
    #[error("exception inside an exception at byte {offset}")]
    NestedException {
        /// Where the second `TC_EXCEPTION` is.
        offset: usize,
    },
    /// A configured limit was exceeded.
    #[error("{limit:?} limit exceeded at byte {offset}")]
    Limit {
        /// Where the item that exceeded it starts.
        offset: usize,
        /// Which limit.
        limit: Limit,
    },
}

/// Why a graph could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// A position refers to a record the stream does not hold.
    #[error("record {0:?} does not exist")]
    UnknownRecord(RecordId),
    /// A position refers to a record of the wrong kind.
    #[error("record {record:?} is a {found} where {expected} is required")]
    WrongRecord {
        /// The record.
        record: RecordId,
        /// The kind the position requires.
        expected: &'static str,
        /// The record's kind.
        found: &'static str,
    },
    /// A record was defined before a `TC_RESET` or an exception and is
    /// referred to after it, when its handle no longer exists.
    #[error("record {0:?} is referred to after the reset that discarded its handle")]
    StaleReference(RecordId),
    /// A descriptor chain loops back on itself.
    #[error("class descriptor {0:?} is its own superclass")]
    CyclicSuperclass(RecordId),
    /// An object or superclass position refers to a descriptor whose own
    /// annotation or superclass is still being written.
    #[error("class descriptor {0:?} is used inside its own definition")]
    IncompleteClassDesc(RecordId),
    /// Class data that does not match the object's descriptors.
    #[error("object {record:?} has class data that does not match its descriptors: {reason}")]
    ClassDataMismatch {
        /// The object.
        record: RecordId,
        /// What does not match.
        reason: &'static str,
    },
    /// Array elements that do not match the array's descriptor.
    #[error("array {0:?} has elements that do not match its descriptor")]
    ArrayMismatch(RecordId),
    /// A length that does not fit the form it is written in.
    #[error("{what} of {length} bytes does not fit its length field")]
    TooLong {
        /// What is too long.
        what: &'static str,
        /// Its length.
        length: usize,
    },
    /// The graph nests deeper than [`crate::Limits::max_depth`].
    #[error("graph nests deeper than the depth limit")]
    TooDeep,
}
