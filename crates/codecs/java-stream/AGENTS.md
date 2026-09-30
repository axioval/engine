# `axioval-java-stream`

A codec for Java object serialization streams: bytes to a graph and back,
byte for byte. See `docs/src/java-stream.md`.

- Clean room. Written only from the public *Java Object Serialization
  Specification* (chapter 6, the stream protocol and its grammar) and from
  streams `tests/java/Generate.java` writes over its own test classes.
  Never consult or copy another implementation of the protocol.
- No class knowledge. The codec decodes field values by the types their
  descriptors declare and keeps custom `writeObject`/`writeExternal` data
  as ordered opaque `Annotation`s. Bindings to particular classes belong to
  the application that owns them, never here. No product or vendor names.
- Depends on `thiserror` and std only; it is a core crate with no
  architecture exemption.
- Exact round trips. `read` then `Stream::to_bytes` returns the input.
  The model keeps every choice a writer can make (`TC_BLOCKDATA` or
  `TC_BLOCKDATALONG`, `TC_STRING` or `TC_LONGSTRING`, modified UTF-8 bytes
  as read, float bits, flag bytes, the aborted bytes before
  `TC_EXCEPTION`). Handles are never stored: the writer defines a record at
  its first position and references it afterwards, so it assigns them in
  grammar order. Any new field must keep that property; the corruption
  test (`tests/hostile.rs`) asserts that every input that reads writes
  back unchanged.
- Canonical forms only where Java never writes anything else: a boolean
  byte other than 0 or 1 is refused rather than normalized. Protocol
  version 1 externalizable data is refused (`ExternalWithoutBlockData`):
  only the class can delimit it.
- Untrusted input: no panics, no `unsafe`, lengths checked against the
  remaining input before allocating, and `Limits` (depth, handles, array
  and string length, total allocation) enforced before memory is taken.
  The writer mirrors the reader's depth accounting, so whatever reads
  under a limit writes under it.
- Tests: `cargo test -p axioval-java-stream`. `tests/fixtures.rs` reads
  every `tests/data/*.ser`; `tests/hand.rs` holds streams built by hand
  from the grammar and with the builder; `tests/hostile.rs` truncates,
  corrupts and fuzzes.
- Regenerating fixtures needs a JDK 17 or newer; from this directory:
  `javac -d ../../../target/java-stream tests/java/Generate.java && java -cp ../../../target/java-stream Generate tests/data`.
  Commit the program and the `.ser` files together. `exception.ser`
  holds a stack trace, so it changes with the program's line numbers.
