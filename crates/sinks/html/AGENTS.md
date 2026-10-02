# `axioval-html`

Self-contained HTML reports of a check run, rendered from a template.

- Depends on `axioval-ir` and `thiserror` only. Never the engine, a source
  adapter, an IFC type or a template engine: the host names the external
  identity scheme (`Options::external_id_scheme`).
- Templates are data. `Template::parse` knows placeholders (`{{slot}}`)
  and literal text, nothing else: never add conditions, loops,
  expressions, includes or any way for a template or a rule package to run
  code. A template that leaves out `summary` or `not-evaluated`, repeats a
  section or names an unknown slot is refused.
- Self-contained: the style is inline (`DEFAULT_STYLE`), and the output
  references no external resource and holds no script. Escape every text
  taken from a report or a project (`escape`).
- Fail closed: not-evaluated outcomes always have their section, an
  unknown table value reads `not evaluated`, and an interval shows both
  bounds with full precision; never round a bound or collapse an
  interval into one number.
- Deterministic: the caller supplies the date; never read the clock.
  Sections follow report order; rules and categories are sorted.
- PDF is the host's step: printing this HTML through a browser. Never add
  a browser or a PDF writer here.
- Run `cargo test -p axioval-html`; `tests/html.rs` renders the default
  and custom templates.
