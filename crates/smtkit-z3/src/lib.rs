//! In-process Z3 backend for `smtkit-core`.
//!
//! This backend is intentionally feature-gated and kept separate from `smtkit-core`.
//! It provides a small adapter:
//! - translate `smtkit-core` terms into Z3 ASTs
//! - solve and extract a projected model for selected variables

use std::collections::HashMap;

use smtkit_core::{Ctx, Op, Sort, Sym, TermId, TermKind};
use z3::ast::Ast;
use z3::{ast, with_z3_config, Config, SatResult, Solver};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelValue {
    Bool(bool),
    Int(i64),
    BitVec { value: u64, width: u32 },
}

pub type Model = HashMap<Sym, ModelValue>;

#[derive(Debug, thiserror::Error)]
pub enum Z3BackendError {
    #[error("unsupported sort: {0:?}")]
    UnsupportedSort(Sort),
    #[error("unsupported operator: {0:?}")]
    UnsupportedOp(Op),
    #[error("type mismatch while translating term")]
    TypeMismatch,
    #[error("model missing value for: {0:?}")]
    MissingValue(Sym),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveStatus {
    Sat,
    Unsat,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct SolveResult<M> {
    pub status: SolveStatus,
    pub model: Option<M>,
}

enum Z3Term {
    Bool(ast::Bool),
    Int(ast::Int),
    Bv(ast::BV),
}

impl Z3Term {
    fn as_bool(&self) -> Option<&ast::Bool> {
        match self {
            Z3Term::Bool(b) => Some(b),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<&ast::Int> {
        match self {
            Z3Term::Int(i) => Some(i),
            _ => None,
        }
    }

    fn as_bv(&self) -> Option<&ast::BV> {
        match self {
            Z3Term::Bv(bv) => Some(bv),
            _ => None,
        }
    }
}

struct Translator<'a> {
    ir: &'a Ctx,
    memo: HashMap<TermId, Z3Term>,
    vars: HashMap<TermId, Sym>,
}

impl<'a> Translator<'a> {
    fn new(ir: &'a Ctx) -> Self {
        Self {
            ir,
            memo: HashMap::new(),
            vars: HashMap::new(),
        }
    }

    fn term(&mut self, t: TermId) -> Result<Z3Term, Z3BackendError> {
        if let Some(v) = self.memo.get(&t) {
            return Ok(match v {
                Z3Term::Bool(b) => Z3Term::Bool(b.clone()),
                Z3Term::Int(i) => Z3Term::Int(i.clone()),
                Z3Term::Bv(bv) => Z3Term::Bv(bv.clone()),
            });
        }

        let out = match self.ir.kind_of(t).clone() {
            TermKind::Var { sym, sort } => {
                self.vars.insert(t, sym.clone());
                match sort {
                    Sort::Bool => Z3Term::Bool(ast::Bool::new_const(sym.0)),
                    Sort::Int => Z3Term::Int(ast::Int::new_const(sym.0)),
                    Sort::BitVec(w) => Z3Term::Bv(ast::BV::new_const(sym.0, w)),
                }
            }
            TermKind::BoolLit(v) => Z3Term::Bool(ast::Bool::from_bool(v)),
            TermKind::IntLit(v) => Z3Term::Int(ast::Int::from_i64(v)),
            TermKind::BvLit { value, width } => Z3Term::Bv(ast::BV::from_u64(value, width)),
            TermKind::App { op, args } => self.app(op, &args)?,
        };

        self.memo.insert(
            t,
            match &out {
                Z3Term::Bool(b) => Z3Term::Bool(b.clone()),
                Z3Term::Int(i) => Z3Term::Int(i.clone()),
                Z3Term::Bv(bv) => Z3Term::Bv(bv.clone()),
            },
        );
        Ok(out)
    }

    fn app(&mut self, op: Op, args: &[TermId]) -> Result<Z3Term, Z3BackendError> {
        match op {
            Op::Not => {
                let a = self.term(args[0])?;
                let b = a.as_bool().ok_or(Z3BackendError::TypeMismatch)?;
                Ok(Z3Term::Bool(b.not()))
            }
            Op::And => {
                let mut bs = Vec::with_capacity(args.len());
                for &a in args {
                    let t = self.term(a)?;
                    bs.push(t.as_bool().ok_or(Z3BackendError::TypeMismatch)?.clone());
                }
                Ok(Z3Term::Bool(ast::Bool::and(&bs)))
            }
            Op::Or => {
                let mut bs = Vec::with_capacity(args.len());
                for &a in args {
                    let t = self.term(a)?;
                    bs.push(t.as_bool().ok_or(Z3BackendError::TypeMismatch)?.clone());
                }
                Ok(Z3Term::Bool(ast::Bool::or(&bs)))
            }
            Op::Eq => {
                let a = self.term(args[0])?;
                let b = self.term(args[1])?;
                Ok(Z3Term::Bool(match (a, b) {
                    (Z3Term::Bool(x), Z3Term::Bool(y)) => x.eq(&y),
                    (Z3Term::Int(x), Z3Term::Int(y)) => x.eq(&y),
                    (Z3Term::Bv(x), Z3Term::Bv(y)) => x.eq(&y),
                    _ => return Err(Z3BackendError::TypeMismatch),
                }))
            }
            Op::Distinct => {
                let sort = self.ir.sort_of(args[0]).clone();
                match sort {
                    Sort::Bool => {
                        let mut xs = Vec::with_capacity(args.len());
                        for &a in args {
                            let t = self.term(a)?;
                            xs.push(t.as_bool().ok_or(Z3BackendError::TypeMismatch)?.clone());
                        }
                        Ok(Z3Term::Bool(ast::Bool::distinct(&xs)))
                    }
                    Sort::Int => {
                        let mut xs = Vec::with_capacity(args.len());
                        for &a in args {
                            let t = self.term(a)?;
                            xs.push(t.as_int().ok_or(Z3BackendError::TypeMismatch)?.clone());
                        }
                        Ok(Z3Term::Bool(ast::Int::distinct(&xs)))
                    }
                    Sort::BitVec(_) => {
                        let mut xs = Vec::with_capacity(args.len());
                        for &a in args {
                            let t = self.term(a)?;
                            xs.push(t.as_bv().ok_or(Z3BackendError::TypeMismatch)?.clone());
                        }
                        Ok(Z3Term::Bool(ast::BV::distinct(&xs)))
                    }
                }
            }
            Op::Lt => {
                let a = self.term(args[0])?;
                let b = self.term(args[1])?;
                Ok(Z3Term::Bool(match (a, b) {
                    (Z3Term::Int(x), Z3Term::Int(y)) => x.lt(&y),
                    (Z3Term::Bv(x), Z3Term::Bv(y)) => x.bvult(&y),
                    _ => return Err(Z3BackendError::TypeMismatch),
                }))
            }
            Op::Le => {
                let a = self.term(args[0])?;
                let b = self.term(args[1])?;
                Ok(Z3Term::Bool(match (a, b) {
                    (Z3Term::Int(x), Z3Term::Int(y)) => x.le(&y),
                    (Z3Term::Bv(x), Z3Term::Bv(y)) => x.bvule(&y),
                    _ => return Err(Z3BackendError::TypeMismatch),
                }))
            }
            Op::Ge => {
                let a = self.term(args[0])?;
                let b = self.term(args[1])?;
                Ok(Z3Term::Bool(match (a, b) {
                    (Z3Term::Int(x), Z3Term::Int(y)) => x.ge(&y),
                    (Z3Term::Bv(x), Z3Term::Bv(y)) => x.bvuge(&y),
                    _ => return Err(Z3BackendError::TypeMismatch),
                }))
            }
            Op::Add => {
                let sort = self.ir.sort_of(args[0]).clone();
                match sort {
                    Sort::Int => {
                        let mut xs = Vec::with_capacity(args.len());
                        for &a in args {
                            let t = self.term(a)?;
                            xs.push(t.as_int().ok_or(Z3BackendError::TypeMismatch)?.clone());
                        }
                        Ok(Z3Term::Int(ast::Int::add(&xs)))
                    }
                    Sort::BitVec(_) => {
                        let mut xs = Vec::with_capacity(args.len());
                        for &a in args {
                            let t = self.term(a)?;
                            xs.push(t.as_bv().ok_or(Z3BackendError::TypeMismatch)?.clone());
                        }
                        let mut it = xs.into_iter();
                        let first = it.next().ok_or(Z3BackendError::TypeMismatch)?;
                        let sum = it.fold(first, |acc, x| acc.bvadd(&x));
                        Ok(Z3Term::Bv(sum))
                    }
                    _ => Err(Z3BackendError::UnsupportedSort(sort)),
                }
            }
            Op::Ite => {
                let c = self.term(args[0])?;
                let c = c.as_bool().ok_or(Z3BackendError::TypeMismatch)?.clone();
                let t = self.term(args[1])?;
                let e = self.term(args[2])?;
                Ok(match (t, e) {
                    (Z3Term::Bool(x), Z3Term::Bool(y)) => Z3Term::Bool(c.ite(&x, &y)),
                    (Z3Term::Int(x), Z3Term::Int(y)) => Z3Term::Int(c.ite(&x, &y)),
                    (Z3Term::Bv(x), Z3Term::Bv(y)) => Z3Term::Bv(c.ite(&x, &y)),
                    _ => Err(Z3BackendError::TypeMismatch)?,
                })
            }
        }
    }
}

/// Solve the conjunction of `assertions` and (if sat) return a projected model for `vars`.
pub fn solve_projected(
    ir: &Ctx,
    assertions: &[TermId],
    vars: &[TermId],
) -> Result<SolveResult<Model>, Z3BackendError> {
    let mut cfg = Config::new();
    cfg.set_model_generation(true);

    with_z3_config(&cfg, || -> Result<SolveResult<Model>, Z3BackendError> {
        let solver = Solver::new();

        let mut tr = Translator::new(ir);
        for &a in assertions {
            let t = tr.term(a)?;
            let b = t.as_bool().ok_or(Z3BackendError::TypeMismatch)?;
            solver.assert(b);
        }

        let status = match solver.check() {
            SatResult::Sat => SolveStatus::Sat,
            SatResult::Unsat => SolveStatus::Unsat,
            SatResult::Unknown => SolveStatus::Unknown,
        };

        if status != SolveStatus::Sat {
            return Ok(SolveResult {
                status,
                model: None,
            });
        }

        let m = solver.get_model().expect("model generation enabled");
        let mut out = Model::new();

        for &v in vars {
            let sym = match ir.kind_of(v) {
                TermKind::Var { sym, .. } => sym.clone(),
                _ => continue,
            };
            let t = tr.term(v)?;
            let val = match (ir.sort_of(v).clone(), t) {
                (Sort::Bool, Z3Term::Bool(b)) => {
                    let ev = m
                        .eval(&b, true)
                        .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?;
                    ModelValue::Bool(
                        ev.as_bool()
                            .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?,
                    )
                }
                (Sort::Int, Z3Term::Int(i)) => {
                    let ev = m
                        .eval(&i, true)
                        .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?;
                    ModelValue::Int(
                        ev.as_i64()
                            .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?,
                    )
                }
                (Sort::BitVec(w), Z3Term::Bv(bv)) => {
                    let ev = m
                        .eval(&bv, true)
                        .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?;
                    ModelValue::BitVec {
                        value: ev
                            .as_u64()
                            .ok_or_else(|| Z3BackendError::MissingValue(sym.clone()))?,
                        width: w,
                    }
                }
                (s, _) => return Err(Z3BackendError::UnsupportedSort(s)),
            };
            out.insert(sym, val);
        }

        Ok(SolveResult {
            status,
            model: Some(out),
        })
    })
}
