//! SMT-LIB2 utilities: s-expressions, script building, and optional solver I/O.
//!
//! This crate is the “SMT-LIB IO” backend in a pySMT-like architecture:
//! - **Serialize** `smtkit-core` terms into SMT-LIB2
//! - **Run** an external solver process (optional; feature-gated)
//!
//! It intentionally does not contain symbolic execution / doc analysis / fuzzy logic.

pub mod emit;
pub mod session;
pub mod sexp;
pub mod smt2;
pub mod solver;
