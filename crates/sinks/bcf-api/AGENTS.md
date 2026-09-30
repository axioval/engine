# `axioval-bcf-api`

BCF API 3.0 (buildingSMART REST) client: pushes a report's topics to a
server and pulls its topics back for decisions.

- Depends on `axioval-bcf` for the topic mapping both ways: push sends what
  `export` writes, pull hands `openbim_bcf::Markup`s to `import_topics`.
  Never map topics a second way here; a finding has one topic GUID however
  it travels.
- A push updates a topic the server has (GET by GUID, then PUT), never
  creates a second. For an undecided finding it keeps the server's status,
  priority, assignee, due date, stage and extra labels: a push must never
  overwrite review state it did not decide. Comments and viewpoints are
  added only when their GUID is missing; viewpoints are immutable in the API.
- Assignee and due date are taken from the exported topic, as in a file.
- Credentials live in `Auth` and `Client` only. Never serialize them,
  never put them in a report, a decisions file or an error message, and
  keep `Debug` redacted.
- No TLS by default; `native-tls` enables HTTPS with the platform's roots.
  Without it an `https` URL is refused (`TlsUnavailable`), never downgraded.
- Tests run against `tests/support/mock.rs`, an in-process server on a
  local port. Never call a real server. The CLI's tests include the same
  file by path.
- Run `cargo test -p axioval-bcf-api` (and `--all-features` for TLS).
