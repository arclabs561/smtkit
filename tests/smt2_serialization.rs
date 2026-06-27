//! Integration tests exercising the `smtkit` facade's pure SMT-LIB2 surface
//! (no solver features required).
//!
//! Every test calls the real public API and asserts against a hand-written
//! SMT-LIB2 oracle. No logic is reimplemented in the assertions.

use smtkit::core::{Ctx, Op, Sort as CoreSort};
use smtkit::sexp::{parse_one, Sexp};
use smtkit::smt2::{t, Script, Sort, Var};

/// Invariant: `Script` serializes its commands in insertion order, one valid
/// SMT-LIB2 command per line, with correct s-expression nesting for sorts and
/// terms. The whole script is the concatenation of those lines.
#[test]
fn script_serializes_full_smt2() {
    let mut s = Script::new();
    s.set_logic("QF_LIA");
    let x = Var::new("x", Sort::Int);
    s.declare_const(&x);
    // (assert (>= x 0))
    s.assert(t::ge(x.sym(), t::int_lit(0)));
    s.check_sat();

    let expected = "(set-logic QF_LIA)\n\
                    (declare-const x Int)\n\
                    (assert (>= x 0))\n\
                    (check-sat)\n";
    assert_eq!(s.to_string(), expected);
}

/// Invariant: `parse_one` parses canonical SMT-LIB2 model output
/// `((x 5) (y 3))` into the expected nested `Sexp` AST, and `Display`
/// re-serializes it byte-for-byte (whitespace normalized to single spaces).
#[test]
fn parse_model_sexp_round_trips_and_structures() {
    let parsed = parse_one("((x 5) (y 3))").unwrap();

    let expected = Sexp::List(vec![
        Sexp::List(vec![Sexp::Atom("x".into()), Sexp::Atom("5".into())]),
        Sexp::List(vec![Sexp::Atom("y".into()), Sexp::Atom("3".into())]),
    ]);
    assert_eq!(parsed, expected);

    // Round-trip back to the canonical string.
    assert_eq!(parsed.to_string(), "((x 5) (y 3))");
}

/// Invariant: a typed core-IR term, asserted into a `Script` via `assert_term`,
/// serializes to the expected SMT-LIB2 s-expression. Exercises both the typed
/// constructors (which enforce sorts) and the IR -> Sexp emit bridge, including
/// the `Op` -> head-symbol mapping (And -> "and", Ge -> ">=", Lt -> "<").
#[test]
fn core_typed_term_emits_expected_smt2() {
    let mut ctx = Ctx::new();
    let x = ctx.var("x", CoreSort::Int);
    let zero = ctx.int_lit(0);
    let ten = ctx.int_lit(10);
    let lb = ctx.app(Op::Ge, vec![x, zero]).unwrap(); // (>= x 0)
    let ub = ctx.app(Op::Lt, vec![x, ten]).unwrap(); // (< x 10)
    let conj = ctx.app(Op::And, vec![lb, ub]).unwrap(); // (and (>= x 0) (< x 10))

    let mut s = Script::new();
    s.assert_term(&ctx, conj);

    assert_eq!(s.to_string(), "(assert (and (>= x 0) (< x 10)))\n");
}

/// Invariant: `Ctx::block_model` builds the standard model-enumeration blocking
/// clause -- a disjunction of disequalities `(or (not (= v_i val_i)) ...)` --
/// which, emitted and asserted, matches the known-correct SMT-LIB2 fragment.
#[test]
fn block_model_emits_blocking_clause() {
    let mut ctx = Ctx::new();
    let x = ctx.var("x", CoreSort::Int);
    let y = ctx.var("y", CoreSort::Int);
    let five = ctx.int_lit(5);
    let three = ctx.int_lit(3);

    let clause = ctx.block_model(&[(x, five), (y, three)]).unwrap();

    let mut s = Script::new();
    s.assert_term(&ctx, clause);

    assert_eq!(s.to_string(), "(assert (or (not (= x 5)) (not (= y 3))))\n");
}
