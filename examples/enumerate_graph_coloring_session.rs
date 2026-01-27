#[cfg(feature = "z3-bin")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use smtkit::smt2::{t, Sort, Var};

    // Enumerate colorings for a triangle graph with 3 colors, using an incremental SMT-LIB session.
    //
    // This demonstrates the “blocking clause” technique (model enumeration):
    // after each satisfying assignment, add a constraint that forces the next model to differ.
    //
    // Requires: an SMT-LIB2 solver binary (e.g. `z3`) and `--features z3-bin`.
    //
    // Override the solver command:
    // - `SMTKIT_SOLVER="z3 -in -smt2"`
    // - `SMTKIT_SOLVER="cvc5 --lang smt2 --incremental"`
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

    let c0 = Var::new("c0", Sort::Int);
    let c1 = Var::new("c1", Sort::Int);
    let c2 = Var::new("c2", Sort::Int);
    for v in [&c0, &c1, &c2] {
        sess.declare_const(&v.name, &v.sort.to_smt2())?;
        // 0 <= ci < 3
        sess.assert_sexp(&t::and(vec![
            t::ge(v.sym(), t::int_lit(0)),
            t::lt(v.sym(), t::int_lit(3)),
        ]))?;
    }
    sess.assert_sexp(&t::distinct(vec![c0.sym(), c1.sym(), c2.sym()]))?;

    let mut count = 0usize;
    loop {
        match sess.check_sat()? {
            smtkit::session::Status::Sat => {
                count += 1;
                // Ask the model for the values of c0,c1,c2.
                let pairs = sess.get_value_pairs(&[c0.sym(), c1.sym(), c2.sym()])?;
                println!("model #{count}: {pairs:?}");

                // Block this exact triple:
                // (or (distinct c0 v0) (distinct c1 v1) (distinct c2 v2))
                let block = t::or(
                    pairs
                        .into_iter()
                        .map(|(k, v)| t::app("distinct", vec![k, v]))
                        .collect::<Vec<_>>(),
                );
                sess.assert_sexp(&block)?;
            }
            smtkit::session::Status::Unsat => {
                println!("total models: {count}");
                break;
            }
            smtkit::session::Status::Unknown => {
                eprintln!("solver returned unknown after {count} models");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "z3-bin"))]
fn main() {
    eprintln!("Enable this example with: cargo run -p smtkit --example enumerate_graph_coloring_session --features z3-bin");
}
