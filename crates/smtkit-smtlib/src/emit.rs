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

/// Render `name` as an SMT-LIB symbol, wrapping it in `|...|` unless it is a
/// simple symbol (non-empty, no leading digit, only letters, digits and
/// ``~!@$%^&*_-+=<>.?/``). A name already wrapped in bars is returned as is.
/// Names containing `|` or `\` have no SMT-LIB spelling; they are wrapped
/// unchanged and the solver will reject them.
pub fn quote_symbol(name: &str) -> String {
    let simple = name.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "~!@$%^&*_-+=<>.?/".contains(c));
    let quoted = name.len() >= 2 && name.starts_with('|') && name.ends_with('|');
    if simple || quoted {
        name.to_string()
    } else {
        format!("|{name}|")
    }
}

pub fn term_to_sexp(ctx: &smtkit_core::Ctx, t: smtkit_core::TermId) -> Sexp {
    match ctx.kind_of(t) {
        smtkit_core::TermKind::Var { sym, .. } => Sexp::atom(quote_symbol(&sym.0)),
        smtkit_core::TermKind::BoolLit(v) => Sexp::atom(if *v { "true" } else { "false" }),
        // SMT-LIB has no negative numerals: `-5` must be `(- 5)`.
        smtkit_core::TermKind::IntLit(v) if *v < 0 => Sexp::list(vec![
            Sexp::atom("-"),
            Sexp::atom(v.unsigned_abs().to_string()),
        ]),
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
                smtkit_core::Op::Add => match ctx.sort_of(t) {
                    smtkit_core::Sort::BitVec(_) => "bvadd",
                    _ => "+",
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use smtkit_core::{Ctx, Op, Sort};

    #[test]
    fn bitvector_add_is_bvadd() {
        let mut ctx = Ctx::new();
        let x = ctx.var("x", Sort::BitVec(8));
        let y = ctx.var("y", Sort::BitVec(8));
        let t = ctx.app(Op::Add, vec![x, y]).unwrap();
        assert_eq!(term_to_sexp(&ctx, t).to_string(), "(bvadd x y)");
    }

    #[test]
    fn integer_add_is_plus() {
        let mut ctx = Ctx::new();
        let x = ctx.var("x", Sort::Int);
        let one = ctx.int_lit(1);
        let t = ctx.app(Op::Add, vec![x, one]).unwrap();
        assert_eq!(term_to_sexp(&ctx, t).to_string(), "(+ x 1)");
    }

    #[test]
    fn negative_integer_literal_is_unary_minus_application() {
        let mut ctx = Ctx::new();
        let n = ctx.int_lit(-5);
        assert_eq!(term_to_sexp(&ctx, n).to_string(), "(- 5)");
        let m = ctx.int_lit(i64::MIN);
        assert_eq!(term_to_sexp(&ctx, m).to_string(), "(- 9223372036854775808)");
        let z = ctx.int_lit(0);
        assert_eq!(term_to_sexp(&ctx, z).to_string(), "0");
    }

    #[test]
    fn symbols_that_are_not_simple_are_bar_quoted() {
        assert_eq!(quote_symbol("x_1"), "x_1");
        assert_eq!(quote_symbol("a.b$c"), "a.b$c");
        assert_eq!(quote_symbol("my var"), "|my var|");
        assert_eq!(quote_symbol("1abc"), "|1abc|");
        assert_eq!(quote_symbol("a(b)"), "|a(b)|");
        assert_eq!(quote_symbol(""), "||");
        assert_eq!(quote_symbol("|already quoted|"), "|already quoted|");

        let mut ctx = Ctx::new();
        let v = ctx.var("my var", Sort::Int);
        assert_eq!(term_to_sexp(&ctx, v).to_string(), "|my var|");
    }
}
