# `codecs/`

Standalone codecs for wire formats. One subdirectory per format.

- A codec reads and writes one format and nothing else: it depends on no
  crate of this tree, knows no engine contract, source format or
  application class, and so is core to the architecture gate with no
  exemption in `scripts/architecture.py`.
- Written from the format's public specification and from data produced
  by the tests' own programs. Never from another implementation's source.
- Round trips are exact: what a codec reads it writes back byte for byte.
  A construct it cannot represent is refused with a typed error, never
  silently normalized.
- Input is untrusted: no panics, every length checked against the
  remaining input before allocating, and configurable hard limits.
- A new codec is a workspace member through `crates/codecs/*`: add it to
  `EXPECTED` in `scripts/check_package_contents.py`, bump
  `EXPECTED_MEMBERS` in `scripts/staging_isolation.py`, and give it its own
  `AGENTS.md`.

## Direct children

- `java-stream/` (`axioval-java-stream`) — Java object serialization streams.
