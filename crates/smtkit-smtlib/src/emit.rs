//! Serialize `smtkit-core` IR into SMT-LIB2 s-expressions.

use crate::sexp::Sexp;

pub fn sort_to_sexp(sort: &smtkit_core::Sort) -> Sexp {
    match sort {
        smtkit_core::Sort::Bool => Sexp::atom("Bool"),
        smtkit_core::Sort::Int => Sexp::atom("Int"),
        smtkit_core::Sort::BitVec(w) => Sexp::list(vec![
            Sexp::atom("_"),
            Sexp::atom("BitVec"),
            Sexp::atom(w.to_string()),
        ]),
    }
}

pub fn term_to_sexp(ctx: &smtkit_core::Ctx, t: smtkit_core::TermId) -> Sexp {
    match ctx.kind_of(t) {
        smtkit_core::TermKind::Var { sym, .. } => Sexp::atom(sym.0.clone()),
        smtkit_core::TermKind::BoolLit(v) => Sexp::atom(if *v { "true" } else { "false" }),
        smtkit_core::TermKind::IntLit(v) => Sexp::atom(v.to_string()),
        smtkit_core::TermKind::BvLit { value, width } => {
            Sexp::atom(format!("(_ bv{} {})", value, width))
        }
        smtkit_core::TermKind::App { op, args } => {
            let head = match op {
                smtkit_core::Op::And => "and",
                smtkit_core::Op::Or => "or",
                smtkit_core::Op::Not => "not",
                smtkit_core::Op::Eq => "=",
                smtkit_core::Op::Distinct => "distinct",
                smtkit_core::Op::Lt => "<",
                smtkit_core::Op::Le => "<=",
                smtkit_core::Op::Ge => ">=",
                smtkit_core::Op::Add => "+",
                smtkit_core::Op::Ite => "ite",
            };
            let mut items = Vec::with_capacity(1 + args.len());
            items.push(Sexp::atom(head));
            for &a in args {
                items.push(term_to_sexp(ctx, a));
            }
            Sexp::list(items)
        }
    }
}
