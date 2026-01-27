use smtkit::smt2::{t, Script, Sort, Var};

fn main() {
    // A tiny “by-example” graph coloring instance:
    // triangle graph (3 nodes fully connected) with 3 colors.
    //
    // SAT: there exists a coloring.
    let mut s = Script::new();
    s.set_logic("QF_LIA");
    s.set_option(":produce-models".to_string(), t::bool_lit(true));

    let c0 = Var::new("c0", Sort::Int);
    let c1 = Var::new("c1", Sort::Int);
    let c2 = Var::new("c2", Sort::Int);
    for v in [&c0, &c1, &c2] {
        s.declare_const(v);
        // 0 <= ci < 3
        s.assert(t::and(vec![
            t::ge(v.sym(), t::int_lit(0)),
            t::lt(v.sym(), t::int_lit(3)),
        ]));
    }

    // All edges in triangle:
    s.assert(t::distinct(vec![c0.sym(), c1.sym(), c2.sym()]));

    s.check_sat();
    s.get_model();

    print!("{}", s.to_string());
}
