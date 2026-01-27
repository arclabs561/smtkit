//! SMT-LIB2 script builder (SMT-LIB backend).
//!
//! This module focuses on emitting valid SMT-LIB2 with a small, explicit API.

use crate::sexp::Sexp;

/// SMT-LIB2 sorts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sort {
    Bool,
    Int,
    BitVec(u32),
}

impl Sort {
    pub fn to_smt2(&self) -> Sexp {
        match self {
            Sort::Bool => Sexp::atom("Bool"),
            Sort::Int => Sexp::atom("Int"),
            Sort::BitVec(w) => Sexp::list(vec![
                Sexp::atom("_"),
                Sexp::atom("BitVec"),
                Sexp::atom(w.to_string()),
            ]),
        }
    }
}

/// A declared variable (as a nullary function).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub name: String,
    pub sort: Sort,
}

impl Var {
    pub fn new(name: impl Into<String>, sort: Sort) -> Self {
        Self {
            name: name.into(),
            sort,
        }
    }

    pub fn sym(&self) -> Sexp {
        Sexp::atom(&self.name)
    }
}

/// A simple SMT-LIB2 script.
#[derive(Clone, Debug, Default)]
pub struct Script {
    items: Vec<Sexp>,
}

impl Script {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn set_logic(&mut self, logic: impl Into<String>) {
        self.items.push(Sexp::list(vec![
            Sexp::atom("set-logic"),
            Sexp::atom(logic.into()),
        ]));
    }

    pub fn set_option(&mut self, key: impl Into<String>, value: Sexp) {
        self.items.push(Sexp::list(vec![
            Sexp::atom("set-option"),
            Sexp::atom(key.into()),
            value,
        ]));
    }

    pub fn comment(&mut self, text: impl AsRef<str>) {
        // SMT-LIB comments start with ';' to end-of-line.
        let mut s = String::from(";");
        s.push_str(text.as_ref());
        self.items.push(Sexp::atom(s));
    }

    pub fn declare_const(&mut self, v: &Var) {
        self.items.push(Sexp::list(vec![
            Sexp::atom("declare-const"),
            v.sym(),
            v.sort.to_smt2(),
        ]));
    }

    pub fn assert(&mut self, term: Sexp) {
        self.items
            .push(Sexp::list(vec![Sexp::atom("assert"), term]));
    }

    /// Assert a typed IR term from `smtkit-core`.
    pub fn assert_term(&mut self, ctx: &smtkit_core::Ctx, t: smtkit_core::TermId) {
        self.assert(crate::emit::term_to_sexp(ctx, t));
    }

    pub fn check_sat(&mut self) {
        self.items.push(Sexp::list(vec![Sexp::atom("check-sat")]));
    }

    pub fn get_model(&mut self) {
        self.items.push(Sexp::list(vec![Sexp::atom("get-model")]));
    }

    pub fn get_unsat_core(&mut self) {
        self.items
            .push(Sexp::list(vec![Sexp::atom("get-unsat-core")]));
    }

    pub fn get_proof(&mut self) {
        self.items.push(Sexp::list(vec![Sexp::atom("get-proof")]));
    }

    pub fn exit(&mut self) {
        self.items.push(Sexp::list(vec![Sexp::atom("exit")]));
    }

    pub fn maximize(&mut self, term: Sexp) {
        self.items
            .push(Sexp::list(vec![Sexp::atom("maximize"), term]));
    }

    pub fn minimize(&mut self, term: Sexp) {
        self.items
            .push(Sexp::list(vec![Sexp::atom("minimize"), term]));
    }
}

impl std::fmt::Display for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep it readable: one item per line. Sexp::Atom is used for comments.
        for it in &self.items {
            writeln!(f, "{}", it)?;
        }
        Ok(())
    }
}

/// Minimal term constructors for SMT-LIB2 `Sexp`.
pub mod t {
    use super::*;

    pub fn sym(name: impl Into<String>) -> Sexp {
        Sexp::atom(name.into())
    }

    pub fn bool_lit(v: bool) -> Sexp {
        Sexp::atom(if v { "true" } else { "false" })
    }

    pub fn int_lit(v: i64) -> Sexp {
        Sexp::atom(v.to_string())
    }

    pub fn bv_lit(value: u64, width: u32) -> Sexp {
        Sexp::atom(format!("(_ bv{} {})", value, width))
    }

    pub fn list(items: impl Into<Vec<Sexp>>) -> Sexp {
        Sexp::list(items)
    }

    pub fn app(f: impl Into<String>, args: impl Into<Vec<Sexp>>) -> Sexp {
        let args: Vec<Sexp> = args.into();
        let mut v = Vec::with_capacity(1 + args.len());
        v.push(Sexp::atom(f.into()));
        v.extend(args);
        Sexp::List(v)
    }

    pub fn eq(a: Sexp, b: Sexp) -> Sexp {
        app("=", vec![a, b])
    }

    pub fn distinct(args: impl Into<Vec<Sexp>>) -> Sexp {
        app("distinct", args)
    }

    pub fn and(args: impl Into<Vec<Sexp>>) -> Sexp {
        app("and", args)
    }

    pub fn or(args: impl Into<Vec<Sexp>>) -> Sexp {
        app("or", args)
    }

    pub fn not(a: Sexp) -> Sexp {
        app("not", vec![a])
    }

    pub fn lt(a: Sexp, b: Sexp) -> Sexp {
        app("<", vec![a, b])
    }

    pub fn le(a: Sexp, b: Sexp) -> Sexp {
        app("<=", vec![a, b])
    }

    pub fn ge(a: Sexp, b: Sexp) -> Sexp {
        app(">=", vec![a, b])
    }

    pub fn add(args: impl Into<Vec<Sexp>>) -> Sexp {
        app("+", args)
    }
}
