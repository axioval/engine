# Java serialization streams

`axioval-java-stream` (`crates/codecs/java-stream`) reads and writes Java
object serialization streams. It implements the stream protocol of the
public *Java Object Serialization Specification* (chapter 6, "Object
Serialization Stream Protocol", and its grammar) and nothing else: it has
no bindings to any host application's classes. It depends on no crate of
this workspace and on no other codec. Formats that store data as such
streams bind their own classes on top of it, in the application that owns
them.

## The graph

`read` turns a stream into a `Stream`:

- `contents`: the top-level items in stream order. An item is an object
  (`Value`), primitive data written outside any object (`BlockData`), a
  `TC_RESET`, or a `TC_EXCEPTION` with the exception object.
- `records`: every item that takes a handle on the wire, in the order the
  stream defines it. A record is a class descriptor (`TC_CLASSDESC`), a
  proxy class descriptor (`TC_PROXYCLASSDESC`), a class (`TC_CLASS`), an
  object (`TC_OBJECT`), an array (`TC_ARRAY`), a string (`TC_STRING`,
  `TC_LONGSTRING`) or an enum constant (`TC_ENUM`).

Every position that holds an object holds a `Value`: `Null` or a
`RecordId`. Handles are not stored. The writer defines a record at the
first position that refers to it and writes `TC_REFERENCE` at every later
one. It assigns the next handle, counting from `baseWireHandle`
(`0x7E0000`), where the grammar assigns `newHandle`. Back-references and
cycles are therefore plain shared ids, and handle numbers always follow
grammar order.

An object carries one `ClassData` entry per descriptor of its superclass
chain, topmost first. Each entry holds the field values its descriptor
declares, decoded by type code (`B C D F I J S Z [ L`). When the
descriptor has `SC_WRITE_METHOD`, the entry also holds the custom data the
class's `writeObject` wrote. An externalizable object (`SC_EXTERNALIZABLE`
with `SC_BLOCK_DATA`) carries a single `External` entry. Custom data,
externalizable data and class annotations are kept as ordered sequences of
opaque block data and objects, because only the class that wrote them
knows their layout.

Strings are kept as their modified UTF-8 bytes (`Mutf8`). They decode to
UTF-16, including lone surrogates, or to a Rust string when they hold
none.

## Exact round trips

`Stream::to_bytes` writes a read stream back byte for byte. The model keeps
every choice a writer can make:

- short or long block data and strings;
- the modified UTF-8 bytes as read, including non-shortest forms;
- float and double bits;
- descriptor flag bytes;
- the bytes of an object a `TC_EXCEPTION` aborted.

The writer's handle table resets where the reader's does: at `TC_RESET`,
and before and after an exception. The test suite reads every stream a
Java program writes over its own test classes. These cover primitives,
strings (long and non-BMP), arrays, cycles, enums, custom `writeObject`
data, `Externalizable`, proxies, subclass chains, class annotations,
resets, `writeUnshared` and an aborted write. Every stream writes back
unchanged. Corrupted variants of those streams either fail to read or also
write back unchanged.

New graphs are built with `Stream::new`, `add` (or `add_string`,
`add_class_desc`, `add_object`), `record_mut` for cycles, and `push`. The
writer checks the graph and fails with a typed `WriteError` when it is
inconsistent: class data that does not match its descriptors, array
elements that do not match their class, a reference across a reset, a
cyclic superclass, or a length that does not fit.

## Limits and refusals

The reader takes untrusted input. It never panics. It checks every length
against the remaining input before allocating, and it enforces `Limits`:

- nesting depth (256 by default, which bounds stack use in reader and
  writer alike);
- handles per handle table;
- array length;
- string length;
- an estimate of the memory the whole graph takes.

A malformed stream is refused with a typed `ReadError` that names the byte
offset. Two constructs are refused although Java may read them:

- externalizable data written with protocol version 1 (no
  `SC_BLOCK_DATA`), which only the class's own `readExternal` can
  delimit;
- a boolean byte other than 0 or 1, which Java never writes.

The grammar assumes that a custom `writeObject` writes its default fields
first, as the specification requires. A stream that breaks this rule
cannot be read by structure alone.
