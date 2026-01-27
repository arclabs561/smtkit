use smtkit::smt2::{t, Script, Sort, Var};

fn main() {
    // Inspired by the “maximize red countries” example:
    // a tiny line graph of 4 nodes with 2 colors (0=red, 1=blue),
    // maximize how many nodes are red subject to proper coloring.
    //
    // NOTE: This is an *optimization* problem; it requires a solver that supports Optimize/maximize.
    let mut s = Script::new();
    s.set_logic("QF_LIA");
    s.set_option(":produce-models".to_string(), t::bool_lit(true));

    let nodes: Vec<Var> = (0..4)
        .map(|i| Var::new(format!("c{i}"), Sort::Int))
        .collect();
    for v in &nodes {
        s.declare_const(v);
        // 0 <= ci < 2
        s.assert(t::and(vec![
            t::ge(v.sym(), t::int_lit(0)),
            t::lt(v.sym(), t::int_lit(2)),
        ]));
    }

    // Line edges: (0-1), (1-2), (2-3)
    for (a, b) in [(0, 1), (1, 2), (2, 3)] {
        s.assert(t::app("distinct", vec![nodes[a].sym(), nodes[b].sym()]));
    }

    // Count reds: sum (ite (= ci 0) 1 0)
    let reds: Vec<_> = nodes
        .iter()
        .map(|v| {
            t::app(
                "ite",
                vec![t::eq(v.sym(), t::int_lit(0)), t::int_lit(1), t::int_lit(0)],
            )
        })
        .collect();
    s.maximize(t::add(reds));

    s.check_sat();
    s.get_model();

    print!("{s}");
}
