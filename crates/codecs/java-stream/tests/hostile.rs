//! Truncated, corrupted and hostile streams: every one is refused with a
//! typed error or read, never a panic, and whatever reads writes back to
//! the very bytes it was read from.

use std::path::PathBuf;

use axioval_java_stream::{Limit, Limits, ReadError, read, read_with};

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut all: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&path).unwrap(),
            )
        })
        .collect();
    all.sort();
    all
}

/// Reads `bytes`; if they read, they must write back unchanged.
fn check(bytes: &[u8], limits: &Limits) -> Result<(), ReadError> {
    let stream = read_with(bytes, limits)?;
    assert_eq!(
        stream.to_bytes_with(limits).expect("a read stream writes"),
        bytes
    );
    Ok(())
}

const HEADER: [u8; 4] = [0xAC, 0xED, 0x00, 0x05];

fn stream(body: &[u8]) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn every_truncation_is_refused_or_reads_a_prefix() {
    for (name, bytes) in fixtures() {
        // Every cut in the first 4 KiB, then a sample of the rest.
        let cuts = (0..bytes.len().min(4096)).chain((4096..bytes.len()).step_by(211));
        for cut in cuts {
            match check(&bytes[..cut], &Limits::default()) {
                Ok(()) | Err(ReadError::Truncated { .. }) => {}
                // A cut inside a top-level block data record leaves a
                // shorter one, and protocol 1 externalizable data is refused
                // wherever it is cut.
                Err(ReadError::ExternalWithoutBlockData { .. })
                    if name.starts_with("protocol1-external") => {}
                Err(error) => panic!("{name} cut at {cut}: {error}"),
            }
        }
    }
}

#[test]
fn corrupted_bytes_are_refused_or_write_back_exactly() {
    let limits = Limits::default();
    for (_, bytes) in fixtures()
        .into_iter()
        .filter(|(_, bytes)| bytes.len() <= 4096)
    {
        for index in 0..bytes.len() {
            for mask in [0x01, 0x10, 0x80, 0xFF] {
                let mut corrupted = bytes.clone();
                corrupted[index] ^= mask;
                let _ = check(&corrupted, &limits);
            }
        }
    }
}

#[test]
fn random_streams_never_panic() {
    // xorshift64*: deterministic, so a failure reproduces.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let limits = Limits {
        max_depth: 64,
        ..Limits::default()
    };
    for _ in 0..20_000 {
        let length = usize::try_from(next() % 64).unwrap();
        // Bias towards type codes and small numbers so records nest.
        let body: Vec<u8> = (0..length)
            .map(|_| match next() % 4 {
                0 => 0x70 + u8::try_from(next() % 15).unwrap(),
                1 => u8::try_from(next() % 4).unwrap(),
                _ => next().to_le_bytes()[0],
            })
            .collect();
        let _ = check(&stream(&body), &limits);
    }
}

#[test]
fn headers_are_checked() {
    assert_eq!(read(&[]), Err(ReadError::Truncated { offset: 0 }));
    assert_eq!(
        read(&[0xCA, 0xFE, 0, 5]),
        Err(ReadError::BadMagic { found: 0xCAFE })
    );
    assert_eq!(
        read(&[0xAC, 0xED, 0, 4]),
        Err(ReadError::UnsupportedVersion { found: 4 })
    );
    assert!(read(&HEADER).unwrap().contents.is_empty());
}

#[test]
fn trailing_garbage_is_refused() {
    let mut bytes =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/primitives.ser"))
            .unwrap();
    bytes.push(0x00);
    assert!(matches!(
        read(&bytes),
        Err(ReadError::UnknownTypeCode { code: 0, .. })
    ));
}

/// A stream body and the error it must be refused with.
type Case = (&'static [u8], fn(&ReadError) -> bool);

#[test]
#[allow(clippy::too_many_lines)]
fn malformed_records_have_typed_errors() {
    let cases: &[Case] = &[
        (&[0x71, 0x00, 0x7E, 0x00, 0x00], |e| {
            matches!(
                e,
                ReadError::UnknownHandle {
                    handle: 0x7E_0000,
                    ..
                }
            )
        }),
        (&[0x71, 0x00, 0x00, 0x00, 0x00], |e| {
            matches!(e, ReadError::UnknownHandle { .. })
        }),
        // A string where an object's class descriptor belongs.
        (&[0x74, 0, 1, b'a', 0x73, 0x71, 0, 0x7E, 0, 0], |e| {
            matches!(
                e,
                ReadError::WrongReference {
                    expected: "class descriptor",
                    found: "string",
                    ..
                }
            )
        }),
        (&[0x73, 0x70], |e| {
            matches!(e, ReadError::UnexpectedNull { .. })
        }),
        (&[0x73, 0x74, 0, 0], |e| {
            matches!(e, ReadError::Unexpected { code: 0x74, .. })
        }),
        (&[0x78], |e| {
            matches!(e, ReadError::Unexpected { code: 0x78, .. })
        }),
        (&[0x74, 0, 1, 0x80], |e| {
            matches!(e, ReadError::InvalidUtf8 { .. })
        }),
        (&[0x7C, 0xFF, 0, 0, 0, 0, 0, 0, 0], |e| {
            matches!(e, ReadError::NegativeLength { .. })
        }),
        (&[0x7C, 0x7F, 0, 0, 0, 0, 0, 0, 0], |e| {
            matches!(
                e,
                ReadError::Limit {
                    limit: Limit::StringLength,
                    ..
                }
            )
        }),
        (&[0x7A, 0xFF, 0xFF, 0xFF, 0xFF], |e| {
            matches!(e, ReadError::NegativeLength { .. })
        }),
        // A field with type code 'X'.
        (
            &[
                0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 1, b'X', 0, 1, b'x', 0x78, 0x70,
            ],
            |e| matches!(e, ReadError::InvalidFieldType { code: b'X', .. }),
        ),
        (
            &[0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0xFF, 0xFF],
            |e| matches!(e, ReadError::NegativeLength { length: -1, .. }),
        ),
        (
            &[
                0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x06, 0, 0, 0x78, 0x70,
            ],
            |e| matches!(e, ReadError::ConflictingFlags { flags: 0x06, .. }),
        ),
        // A descriptor that is its own superclass.
        (
            &[
                0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x71, 0, 0x7E, 0, 0,
            ],
            |e| matches!(e, ReadError::IncompleteClassDesc { .. }),
        ),
        // An object of a class inside that class's own annotation.
        (
            &[
                0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x73, 0x71, 0, 0x7E, 0, 0,
            ],
            |e| matches!(e, ReadError::IncompleteClassDesc { .. }),
        ),
        // A boolean field holding 2.
        (
            &[
                0x73, 0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 1, b'Z', 0, 1, b'z', 0x78,
                0x70, 2,
            ],
            |e| matches!(e, ReadError::InvalidBoolean { value: 2, .. }),
        ),
        // An array whose class is no array class.
        (
            &[
                0x75, 0x72, 0, 1, b'A', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0, 0, 0, 0,
            ],
            |e| matches!(e, ReadError::InvalidArrayClass { .. }),
        ),
        // An int[] of 2^31 - 1 elements in a few bytes: refused before allocating.
        (
            &[
                0x75, 0x72, 0, 2, b'[', b'I', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0x7F,
                0xFF, 0xFF, 0xFF,
            ],
            |e| {
                matches!(
                    e,
                    ReadError::Limit {
                        limit: Limit::ArrayLength,
                        ..
                    }
                )
            },
        ),
        (
            &[
                0x75, 0x72, 0, 2, b'[', b'I', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0x00,
                0xFF, 0xFF, 0xFF,
            ],
            |e| matches!(e, ReadError::Truncated { .. }),
        ),
        (
            &[
                0x75, 0x72, 0, 2, b'[', b'I', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0xFF,
                0xFF, 0xFF, 0xFF,
            ],
            |e| matches!(e, ReadError::NegativeLength { .. }),
        ),
        (&[0x7D, 0x7F, 0xFF, 0xFF, 0xFF], |e| {
            matches!(e, ReadError::TooManyInterfaces { .. })
        }),
        // TC_RESET inside an object array.
        (
            &[
                0x75, 0x72, 0, 2, b'[', b'L', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0, 0,
                0, 1, 0x79,
            ],
            |e| matches!(e, ReadError::NestedReset { .. }),
        ),
        // An exception whose throwable is itself aborted.
        (&[0x7B, 0x7B], |e| {
            matches!(e, ReadError::NestedException { .. })
        }),
        // An enum constant named by null.
        (
            &[
                0x7E, 0x72, 0, 1, b'E', 0, 0, 0, 0, 0, 0, 0, 0, 0x12, 0, 0, 0x78, 0x70, 0x70,
            ],
            |e| {
                matches!(
                    e,
                    ReadError::UnexpectedNull {
                        expected: "a string",
                        ..
                    }
                )
            },
        ),
    ];
    for (body, expected) in cases {
        let error = read(&stream(body)).expect_err("malformed");
        assert!(expected(&error), "{body:02x?}: {error:?}");
    }
}

#[test]
fn protocol_1_externalizable_data_is_refused() {
    let body = [
        0x73, 0x72, 0, 1, b'X', 0, 0, 0, 0, 0, 0, 0, 0, 0x04, 0, 0, 0x78, 0x70, 1, 2, 3,
    ];
    assert!(matches!(
        read(&stream(&body)),
        Err(ReadError::ExternalWithoutBlockData { offset: 4 })
    ));
}

/// `depth` nested `Object[]`s, the innermost holding null.
fn nested_arrays(depth: usize) -> Vec<u8> {
    let mut body = vec![0x75, 0x72, 0, 19];
    body.extend_from_slice(b"[Ljava.lang.Object;");
    body.extend([0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70, 0, 0, 0, 1]);
    for _ in 1..depth {
        body.extend([0x75, 0x71, 0, 0x7E, 0, 0, 0, 0, 0, 1]);
    }
    body.push(0x70);
    stream(&body)
}

/// `depth` objects of a class with `writeObject`, each in the custom data
/// of the one before.
fn nested_custom_data(depth: usize) -> Vec<u8> {
    let mut body = vec![
        0x73, 0x72, 0, 1, b'N', 0, 0, 0, 0, 0, 0, 0, 0, 0x03, 0, 0, 0x78, 0x70,
    ];
    for _ in 1..depth {
        body.extend([0x73, 0x71, 0, 0x7E, 0, 0]);
    }
    body.extend(std::iter::repeat_n(0x78, depth));
    stream(&body)
}

#[test]
fn nested_custom_data_is_bounded_by_the_depth_limit() {
    let limits = Limits::default();
    check(&nested_custom_data(limits.max_depth - 1), &limits).unwrap();
    assert!(matches!(
        read(&nested_custom_data(limits.max_depth)),
        Err(ReadError::Limit {
            limit: Limit::Depth,
            ..
        })
    ));
}

#[test]
fn nesting_is_bounded_by_the_depth_limit() {
    let limits = Limits::default();
    // The deepest graph the default limit admits reads and writes on a
    // test thread's stack; the class descriptor takes one more level.
    check(&nested_arrays(limits.max_depth - 1), &limits).unwrap();
    assert!(matches!(
        read(&nested_arrays(limits.max_depth)),
        Err(ReadError::Limit {
            limit: Limit::Depth,
            ..
        })
    ));
    assert!(matches!(
        read(&nested_arrays(100_000)),
        Err(ReadError::Limit {
            limit: Limit::Depth,
            ..
        })
    ));
}

#[test]
fn handles_and_allocation_are_bounded() {
    // A thousand one-character strings.
    let mut body = Vec::new();
    for _ in 0..1000 {
        body.extend([0x74, 0, 1, b's']);
    }
    let bytes = stream(&body);
    check(&bytes, &Limits::default()).unwrap();
    let few_handles = Limits {
        max_handles: 999,
        ..Limits::default()
    };
    assert!(matches!(
        read_with(&bytes, &few_handles),
        Err(ReadError::Limit {
            limit: Limit::Handles,
            ..
        })
    ));
    // A reset empties the handle table, so the same limit admits both halves.
    let mut reset = body[..2000].to_vec();
    reset.push(0x79);
    reset.extend_from_slice(&body[2000..]);
    check(&stream(&reset), &few_handles).unwrap();
    let little_memory = Limits {
        max_allocation: 10_000,
        ..Limits::default()
    };
    assert!(matches!(
        read_with(&bytes, &little_memory),
        Err(ReadError::Limit {
            limit: Limit::Allocation,
            ..
        })
    ));
}

#[test]
fn a_long_superclass_chain_is_charged_per_object() {
    // 2000 descriptors, each the superclass of the next, then 2000 objects
    // of the last: a few bytes each, but 2000 class data entries apiece.
    let mut body = vec![
        0x72, 0, 1, b'C', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x70,
    ];
    for handle in 0..1999u32 {
        body.extend([
            0x72, 0, 1, b'C', 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0x78, 0x71,
        ]);
        body.extend((0x7E_0000 + handle).to_be_bytes());
    }
    let object = |body: &mut Vec<u8>| {
        body.extend([0x73, 0x71]);
        body.extend((0x7E_0000u32 + 1999).to_be_bytes());
    };
    let mut few = body.clone();
    for _ in 0..10 {
        object(&mut few);
    }
    check(&stream(&few), &Limits::default()).unwrap();
    for _ in 0..2000 {
        object(&mut body);
    }
    // About 4 million entries: refused long before they are allocated.
    let limits = Limits {
        max_allocation: 64 << 20,
        ..Limits::default()
    };
    assert!(matches!(
        read_with(&stream(&body), &limits),
        Err(ReadError::Limit {
            limit: Limit::Allocation,
            ..
        })
    ));
}
