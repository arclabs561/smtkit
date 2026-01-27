//! Facade crate for SMT constraint tooling.
//!
//! This crate is the stable entrypoint users depend on. It re-exports:
//! - `smtkit-core`: typed, backend-agnostic constraint IR
//! - `smtkit-smtlib`: SMT-LIB2 serialization and optional solver I/O
//!
//! ## DX-first solver guidance
//!
//! - Prefer **`z3-bin`** for most users: it runs an external `z3` binary and avoids native headers/toolchains.
//! - Use **`z3-inproc`** only when you specifically want in-process Z3; it may require system Z3 headers
//!   (and/or a CMake toolchain if you enable bundled builds in the underlying Z3 crate).

pub use smtkit_smtlib::session;
pub use smtkit_smtlib::{sexp, smt2, solver};

#[cfg(feature = "z3-inproc")]
pub mod z3 {
    //! In-process Z3 backend (feature-gated).
    //!
    //! This is a convenience re-export of the `smtkit-z3` adapter crate.

    pub use smtkit_z3::{Model, ModelValue, Z3BackendError};

    use crate::{SolveResult, SolveStatus};
    use smtkit_core::{Ctx, TermId};

    /// Solve a set of `assertions` and (if sat) return a projected model for `vars`.
    pub fn solve_projected(
        ctx: &Ctx,
        assertions: &[TermId],
        vars: &[TermId],
    ) -> Result<SolveResult<Model>, Z3BackendError> {
        let r = smtkit_z3::solve_projected(ctx, assertions, vars)?;
        let status = match r.status {
            smtkit_z3::SolveStatus::Sat => SolveStatus::Sat,
            smtkit_z3::SolveStatus::Unsat => SolveStatus::Unsat,
            smtkit_z3::SolveStatus::Unknown => SolveStatus::Unknown,
        };
        Ok(SolveResult {
            status,
            model: r.model,
        })
    }
}

/// A small “SMT-LIB over stdio” session surface, stable at the `smtkit` facade.
///
/// This is intentionally **SMT-LIB shaped** (it uses `sexp::Sexp`) and is not intended
/// to be backend-agnostic across non-SMT-LIB APIs.
pub trait SmtlibSessionLike {
    type Error;

    fn set_logic(&mut self, logic: &str) -> Result<(), Self::Error>;
    fn set_timeout_ms(&mut self, ms: u64) -> Result<(), Self::Error>;
    fn set_random_seed(&mut self, seed: u64) -> Result<(), Self::Error>;
    fn set_produce_models(&mut self, enabled: bool) -> Result<(), Self::Error>;
    fn set_print_success(&mut self, enabled: bool) -> Result<(), Self::Error>;

    fn push(&mut self) -> Result<(), Self::Error>;
    fn pop(&mut self, n: u32) -> Result<(), Self::Error>;

    fn declare_const(&mut self, name: &str, sort: &sexp::Sexp) -> Result<(), Self::Error>;
    fn assert_sexp(&mut self, term: &sexp::Sexp) -> Result<(), Self::Error>;

    fn check_sat(&mut self) -> Result<session::Status, Self::Error>;
    fn check_sat_assuming(
        &mut self,
        assumptions: &[sexp::Sexp],
    ) -> Result<session::Status, Self::Error>;

    fn get_value_pairs(
        &mut self,
        terms: &[sexp::Sexp],
    ) -> Result<Vec<(sexp::Sexp, sexp::Sexp)>, Self::Error>;
}

impl SmtlibSessionLike for smtkit_smtlib::session::SmtlibSession {
    type Error = smtkit_smtlib::session::SessionError;

    fn set_logic(&mut self, logic: &str) -> Result<(), Self::Error> {
        self.set_logic(logic)
    }

    fn set_timeout_ms(&mut self, ms: u64) -> Result<(), Self::Error> {
        self.set_timeout_ms(ms)
    }

    fn set_random_seed(&mut self, seed: u64) -> Result<(), Self::Error> {
        self.set_random_seed(seed)
    }

    fn set_produce_models(&mut self, enabled: bool) -> Result<(), Self::Error> {
        self.set_produce_models(enabled)
    }

    fn set_print_success(&mut self, enabled: bool) -> Result<(), Self::Error> {
        self.set_print_success(enabled)
    }

    fn push(&mut self) -> Result<(), Self::Error> {
        self.push()
    }

    fn pop(&mut self, n: u32) -> Result<(), Self::Error> {
        self.pop(n)
    }

    fn declare_const(&mut self, name: &str, sort: &sexp::Sexp) -> Result<(), Self::Error> {
        self.declare_const(name, sort)
    }

    fn assert_sexp(&mut self, term: &sexp::Sexp) -> Result<(), Self::Error> {
        self.assert_sexp(term)
    }

    fn check_sat(&mut self) -> Result<session::Status, Self::Error> {
        self.check_sat()
    }

    fn check_sat_assuming(
        &mut self,
        assumptions: &[sexp::Sexp],
    ) -> Result<session::Status, Self::Error> {
        self.check_sat_assuming(assumptions)
    }

    fn get_value_pairs(
        &mut self,
        terms: &[sexp::Sexp],
    ) -> Result<Vec<(sexp::Sexp, sexp::Sexp)>, Self::Error> {
        self.get_value_pairs(terms)
    }
}

/// Typed, backend-agnostic constraint IR.
pub mod core {
    pub use smtkit_core::*;
}

/// Backend-agnostic solver result shape (status + optional model payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveStatus {
    Sat,
    Unsat,
    Unknown,
}

/// A minimal solver result. `model` is backend-defined (e.g. an SMT-LIB s-expression).
#[derive(Clone, Debug)]
pub struct SolveResult<M> {
    pub status: SolveStatus,
    pub model: Option<M>,
}

#[cfg(feature = "z3-bin")]
#[derive(Debug, thiserror::Error)]
pub enum Z3SolveError {
    #[error("z3 invocation failed")]
    Run(#[from] solver::SolverError),
    #[error("failed to parse z3 output")]
    Parse(#[from] solver::ParseSolverOutputError),
}

/// Run Z3 (external binary) on an SMT-LIB2 script string and parse status/model.
///
/// Enabled by the `z3-bin` feature.
#[cfg(feature = "z3-bin")]
pub fn solve_z3_smt2(
    smt2: &str,
    extra_args: &[&str],
) -> Result<SolveResult<sexp::Sexp>, Z3SolveError> {
    let (stdout, stderr) = solver::run_z3_stdin(smt2, extra_args)?;
    let parsed = solver::parse_z3_output(&stdout, &stderr)?;
    let status = match parsed.status {
        solver::Status::Sat => SolveStatus::Sat,
        solver::Status::Unsat => SolveStatus::Unsat,
        solver::Status::Unknown => SolveStatus::Unknown,
    };
    Ok(SolveResult {
        status,
        model: parsed.model,
    })
}
