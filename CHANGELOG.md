# Changelog

## [Unreleased]

### Fixed

- `smt2::t::sym` quotes symbol names consistently with `Var::sym`, including
  spaces and parentheses, and rejects unrepresentable names using the existing
  panic convention. Callers supplying raw SMT-LIB spellings can use `Sexp::atom`.
- Script and session `set_logic` helpers render the logic name as a single
  symbol using the existing symbol formatter. Unrepresentable names panic before
  changing the script or writing to the solver, matching the declaration helpers.
- Script comments prefix every line after either CR or LF, keeping multiline
  diagnostic text in comments.
- CI runs the field-serialization regressions with an external Z3 solver.

## smtkit 0.2.0 and smtkit-smtlib 0.2.0 - 2026-10-09

### Changed

- `smtkit_smtlib::session::SessionError` and `smtkit_smtlib::sexp::ParseError`
  are `#[non_exhaustive]`. Matches on them need a wildcard arm.
- The s-expression parser caps nesting at 512 and returns
  `ParseError::TooDeep`. Before, deeply nested input (100,000 parentheses)
  overflowed the stack and aborted the process.
- Solver sessions bound response bytes, unterminated lines, the stdout queue,
  writes and teardown. Oversized output terminates the solver process and
  poisons the session; the per-response limit is configurable.
- The `smtkit` facade requires smtkit-smtlib 0.2.0.

### Fixed

- SMT-LIB emission writes bit-vector addition as `bvadd` (it was `+`),
  negative integers as `(- N)` (they were `-N`, which SMT-LIB reads as a
  symbol), including from the `t::int_lit` script builder, and quotes symbols
  that need `|...|`.

## 0.1.0

- Initial crates.io release.
