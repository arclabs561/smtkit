#[cfg(feature = "z3-inproc")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use smtkit::core::{Ctx, Op, Sort, TermId, TypeError};

    fn and(ctx: &mut Ctx, args: Vec<TermId>) -> Result<TermId, TypeError> {
        ctx.app(Op::And, args)
    }
    fn distinct(ctx: &mut Ctx, args: Vec<TermId>) -> Result<TermId, TypeError> {
        ctx.app(Op::Distinct, args)
    }
    fn ge(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Ge, vec![a, b])
    }
    fn lt(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Lt, vec![a, b])
    }

    // Triangle graph coloring with 3 colors, enumerated via blocking clauses.
    let mut ctx = Ctx::new();
    let c0 = ctx.var("c0", Sort::Int);
    let c1 = ctx.var("c1", Sort::Int);
    let c2 = ctx.var("c2", Sort::Int);
    let vars = [c0, c1, c2];

    // 0 <= ci < 3
    let zero = ctx.int_lit(0);
    let three = ctx.int_lit(3);
    let mut assertions = Vec::new();
    for &v in &vars {
        let lo = ge(&mut ctx, v, zero)?;
        let hi = lt(&mut ctx, v, three)?;
        assertions.push(and(&mut ctx, vec![lo, hi])?);
    }
    assertions.push(distinct(&mut ctx, vec![c0, c1, c2])?);

    let mut count = 0usize;
    loop {
        let r = smtkit::z3::solve_projected(&ctx, &assertions, &vars)?;
        match r.status {
            smtkit::SolveStatus::Sat => {
                count += 1;
                let m = r.model.expect("sat implies model");
                println!("model #{count}: {m:?}");

                // Build blocking clause from projected values.
                let mut assigns = Vec::new();
                for &v in &vars {
                    let sym = match ctx.kind_of(v) {
                        smtkit::core::TermKind::Var { sym, .. } => sym.clone(),
                        _ => continue,
                    };
                    let mv = m.get(&sym).expect("projected var present");
                    let lit = match *mv {
                        smtkit::z3::ModelValue::Int(n) => ctx.int_lit(n),
                        smtkit::z3::ModelValue::Bool(b) => ctx.bool_lit(b),
                        smtkit::z3::ModelValue::BitVec { value, width } => ctx.bv_lit(value, width),
                    };
                    assigns.push((v, lit));
                }
                let block = ctx.block_model(&assigns)?;
                assertions.push(block);
            }
            smtkit::SolveStatus::Unsat => {
                println!("total models: {count}");
                break;
            }
            smtkit::SolveStatus::Unknown => {
                eprintln!("solver returned unknown after {count} models");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "z3-inproc"))]
fn main() {
    eprintln!("Enable this example with: cargo run -p smtkit --example enumerate_graph_coloring_inproc --features z3-inproc-gh-release");
}
