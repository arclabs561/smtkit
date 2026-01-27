//! Solver-agnostic, typed constraint IR.
//!
//! This crate is intentionally backend-agnostic:
//! - No solver bindings
//! - No SMT-LIB process I/O
//! - Pure term construction + typing utilities
//!
//! A higher layer (e.g. an SMT-LIB backend crate) is expected to:
//! - serialize the IR into SMT-LIB2 (or a native solver API)
//! - run a concrete solver and translate results back

use std::collections::{BTreeSet, HashMap};

/// A symbol/name used to identify variables.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub String);

impl From<&str> for Sym {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Sym {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Sorts supported by the core IR.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Sort {
    Bool,
    Int,
    BitVec(u32),
}

/// A handle to a term in an arena.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TermId(u32);

impl TermId {
    fn new(i: usize) -> Self {
        Self(i as u32)
    }
}

/// Built-in operators. (Higher layers may also support uninterpreted functions.)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    And,
    Or,
    Not,
    Eq,
    Distinct,
    Lt,
    Le,
    Ge,
    Add,
    Ite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermKind {
    Var { sym: Sym, sort: Sort },
    BoolLit(bool),
    IntLit(i64),
    BvLit { value: u64, width: u32 },
    App { op: Op, args: Vec<TermId> },
}

#[derive(Clone, Debug)]
struct TermNode {
    kind: TermKind,
    sort: Sort,
}

#[derive(Debug, thiserror::Error)]
pub enum TypeError {
    #[error("expected Bool, got {0:?}")]
    ExpectedBool(Sort),
    #[error("expected Int, got {0:?}")]
    ExpectedInt(Sort),
    #[error("expected BitVec({expected}), got {got:?}")]
    ExpectedBitVec { expected: u32, got: Sort },
    #[error("sort mismatch: expected {expected:?}, got {got:?}")]
    SortMismatch { expected: Sort, got: Sort },
    #[error("arity error for {op:?}: expected {expected}, got {got}")]
    Arity { op: Op, expected: usize, got: usize },
    #[error("operator {op:?} requires at least {min} args, got {got}")]
    MinArity { op: Op, min: usize, got: usize },
    #[error("empty argument list is not allowed for {op:?}")]
    EmptyArgs { op: Op },
}

/// A typed term arena with constructors that enforce sorts.
#[derive(Default, Debug)]
pub struct Ctx {
    nodes: Vec<TermNode>,
}

impl Ctx {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn sort_of(&self, t: TermId) -> &Sort {
        &self.nodes[t.0 as usize].sort
    }

    pub fn kind_of(&self, t: TermId) -> &TermKind {
        &self.nodes[t.0 as usize].kind
    }

    pub fn var(&mut self, sym: impl Into<Sym>, sort: Sort) -> TermId {
        self.push(
            TermKind::Var {
                sym: sym.into(),
                sort: sort.clone(),
            },
            sort,
        )
    }

    pub fn bool_lit(&mut self, v: bool) -> TermId {
        self.push(TermKind::BoolLit(v), Sort::Bool)
    }

    pub fn int_lit(&mut self, v: i64) -> TermId {
        self.push(TermKind::IntLit(v), Sort::Int)
    }

    pub fn bv_lit(&mut self, value: u64, width: u32) -> TermId {
        self.push(TermKind::BvLit { value, width }, Sort::BitVec(width))
    }

    pub fn app(&mut self, op: Op, args: Vec<TermId>) -> Result<TermId, TypeError> {
        let sort = self.infer_app_sort(&op, &args)?;
        Ok(self.push(TermKind::App { op, args }, sort))
    }

    /// Shorthand for `not (= a b)` (typed).
    pub fn neq(&mut self, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        let eq = self.app(Op::Eq, vec![a, b])?;
        self.app(Op::Not, vec![eq])
    }

    /// Build a standard *blocking clause* for model enumeration:
    ///
    /// \[
    /// \bigvee_i (v_i \ne value_i)
    /// \]
    ///
    /// The caller controls which variables are projected by choosing which pairs to include.
    pub fn block_model(&mut self, assignments: &[(TermId, TermId)]) -> Result<TermId, TypeError> {
        if assignments.is_empty() {
            return Err(TypeError::EmptyArgs { op: Op::Or });
        }
        let mut disj = Vec::with_capacity(assignments.len());
        for &(v, val) in assignments {
            disj.push(self.neq(v, val)?);
        }
        self.app(Op::Or, disj)
    }

    fn push(&mut self, kind: TermKind, sort: Sort) -> TermId {
        let id = TermId::new(self.nodes.len());
        self.nodes.push(TermNode { kind, sort });
        id
    }

    fn infer_app_sort(&self, op: &Op, args: &[TermId]) -> Result<Sort, TypeError> {
        match op {
            Op::Not => {
                if args.len() != 1 {
                    return Err(TypeError::Arity {
                        op: op.clone(),
                        expected: 1,
                        got: args.len(),
                    });
                }
                self.require_bool(args[0])?;
                Ok(Sort::Bool)
            }
            Op::And | Op::Or => {
                if args.is_empty() {
                    return Err(TypeError::EmptyArgs { op: op.clone() });
                }
                for &a in args {
                    self.require_bool(a)?;
                }
                Ok(Sort::Bool)
            }
            Op::Eq => {
                if args.len() != 2 {
                    return Err(TypeError::Arity {
                        op: op.clone(),
                        expected: 2,
                        got: args.len(),
                    });
                }
                self.require_same_sort(args[0], args[1])?;
                Ok(Sort::Bool)
            }
            Op::Distinct => {
                if args.len() < 2 {
                    return Err(TypeError::MinArity {
                        op: op.clone(),
                        min: 2,
                        got: args.len(),
                    });
                }
                let s0 = self.sort_of(args[0]).clone();
                for &a in &args[1..] {
                    self.require_sort(a, &s0)?;
                }
                Ok(Sort::Bool)
            }
            Op::Lt | Op::Le | Op::Ge => {
                if args.len() != 2 {
                    return Err(TypeError::Arity {
                        op: op.clone(),
                        expected: 2,
                        got: args.len(),
                    });
                }
                self.require_int(args[0])?;
                self.require_int(args[1])?;
                Ok(Sort::Bool)
            }
            Op::Add => {
                if args.len() < 2 {
                    return Err(TypeError::MinArity {
                        op: op.clone(),
                        min: 2,
                        got: args.len(),
                    });
                }
                let s0 = self.sort_of(args[0]).clone();
                match s0 {
                    Sort::Int => {
                        for &a in &args[1..] {
                            self.require_int(a)?;
                        }
                        Ok(Sort::Int)
                    }
                    Sort::BitVec(w) => {
                        for &a in &args[1..] {
                            self.require_bv(a, w)?;
                        }
                        Ok(Sort::BitVec(w))
                    }
                    Sort::Bool => Err(TypeError::ExpectedInt(Sort::Bool)),
                }
            }
            Op::Ite => {
                if args.len() != 3 {
                    return Err(TypeError::Arity {
                        op: op.clone(),
                        expected: 3,
                        got: args.len(),
                    });
                }
                self.require_bool(args[0])?;
                self.require_same_sort(args[1], args[2])?;
                Ok(self.sort_of(args[1]).clone())
            }
        }
    }

    fn require_bool(&self, t: TermId) -> Result<(), TypeError> {
        match self.sort_of(t) {
            Sort::Bool => Ok(()),
            s => Err(TypeError::ExpectedBool(s.clone())),
        }
    }

    fn require_int(&self, t: TermId) -> Result<(), TypeError> {
        match self.sort_of(t) {
            Sort::Int => Ok(()),
            s => Err(TypeError::ExpectedInt(s.clone())),
        }
    }

    fn require_bv(&self, t: TermId, w: u32) -> Result<(), TypeError> {
        match self.sort_of(t) {
            Sort::BitVec(got_w) if *got_w == w => Ok(()),
            s => Err(TypeError::ExpectedBitVec {
                expected: w,
                got: s.clone(),
            }),
        }
    }

    fn require_sort(&self, t: TermId, expected: &Sort) -> Result<(), TypeError> {
        let got = self.sort_of(t);
        if got == expected {
            Ok(())
        } else {
            Err(TypeError::SortMismatch {
                expected: expected.clone(),
                got: got.clone(),
            })
        }
    }

    fn require_same_sort(&self, a: TermId, b: TermId) -> Result<(), TypeError> {
        let sa = self.sort_of(a).clone();
        let sb = self.sort_of(b).clone();
        if sa == sb {
            Ok(())
        } else {
            Err(TypeError::SortMismatch {
                expected: sa,
                got: sb,
            })
        }
    }

    /// Return the set of free variables (symbols) in `root`.
    pub fn free_vars(&self, root: TermId) -> BTreeSet<Sym> {
        let mut out = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(t) = stack.pop() {
            match self.kind_of(t) {
                TermKind::Var { sym, .. } => {
                    out.insert(sym.clone());
                }
                TermKind::App { args, .. } => {
                    stack.extend(args.iter().copied());
                }
                TermKind::BoolLit(_) | TermKind::IntLit(_) | TermKind::BvLit { .. } => {}
            }
        }
        out
    }

    /// Capture-avoiding substitution by symbol (best-effort; variables are identified by `Sym`).
    ///
    /// This rebuilds a new term DAG in the same arena.
    pub fn substitute(
        &mut self,
        root: TermId,
        subst: &HashMap<Sym, TermId>,
    ) -> Result<TermId, TypeError> {
        fn go(
            ctx: &mut Ctx,
            t: TermId,
            subst: &HashMap<Sym, TermId>,
            memo: &mut HashMap<TermId, TermId>,
        ) -> Result<TermId, TypeError> {
            if let Some(&hit) = memo.get(&t) {
                return Ok(hit);
            }
            let out = match ctx.kind_of(t).clone() {
                TermKind::Var { sym, sort } => subst
                    .get(&sym)
                    .copied()
                    .unwrap_or_else(|| ctx.var(sym, sort)),
                TermKind::BoolLit(v) => ctx.bool_lit(v),
                TermKind::IntLit(v) => ctx.int_lit(v),
                TermKind::BvLit { value, width } => ctx.bv_lit(value, width),
                TermKind::App { op, args } => {
                    let mut new_args = Vec::with_capacity(args.len());
                    for a in args {
                        new_args.push(go(ctx, a, subst, memo)?);
                    }
                    ctx.app(op, new_args)?
                }
            };
            memo.insert(t, out);
            Ok(out)
        }

        let mut memo = HashMap::new();
        go(self, root, subst, &mut memo)
    }
}
