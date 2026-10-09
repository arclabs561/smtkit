# Changelog

## [Unreleased]

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

