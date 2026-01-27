#[cfg(feature = "z3-bin")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use smtkit::smt2::t;

    // Small “session smoke” showing determinism hooks:
    // - solver-side timeout
    // - solver-side random seed
    //
    // This example is intentionally tiny and will print a “next move” message if no solver is found.

    let (mut sess, used) = match smtkit::session::spawn_auto() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to start solver session: {e}");
            eprintln!("next move:");
            eprintln!("- install `z3` or `cvc5`, or");
            eprintln!("- set `SMTKIT_SOLVER` to an explicit command line, e.g. `SMTKIT_SOLVER=\"z3 -in -smt2\"`");
            return Err(Box::new(e));
        }
    };
    eprintln!("using solver: {used}");

    sess.set_logic("QF_LIA")?;
    sess.set_print_success(false)?;
    sess.set_produce_models(true)?;

    // Determinism hooks (best-effort; solver-dependent).
    sess.set_timeout_ms(2_000)?;
    sess.set_random_seed(0)?;

    // Trivial sat problem with one variable.
    sess.declare_const("x", &smtkit::smt2::Sort::Int.to_smt2())?;
    sess.assert_sexp(&t::eq(t::sym("x"), t::int_lit(0)))?;

    let st = sess.check_sat()?;
    println!("status: {st:?}");
    if st == smtkit::session::Status::Sat {
        let pairs = sess.get_value_pairs(&[t::sym("x")])?;
        println!("model: {pairs:?}");
    }

    Ok(())
}

#[cfg(not(feature = "z3-bin"))]
fn main() {
    eprintln!("Enable this example with: cargo run -p smtkit --example session_determinism_hooks --features z3-bin");
}
