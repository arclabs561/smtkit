/// Multi-objective optimization via SMT enumeration + Pareto frontier.
///
/// Problem: assign 4 workers to 4 tasks (each worker does exactly one task).
/// Three objectives (all minimized):
///   - total cost
///   - max latency (the slowest task)
///   - total risk
///
/// The solver enumerates feasible assignments. Each solution is fed to
/// `pare::ParetoFrontier` to track the non-dominated set. A blocking clause
/// prevents re-visiting the same assignment.
#[cfg(feature = "z3-inproc")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pare::{Direction, ParetoFrontier};
    use smtkit::core::{Ctx, Op, Sort, TermId, TypeError};

    fn and(ctx: &mut Ctx, args: Vec<TermId>) -> Result<TermId, TypeError> {
        ctx.app(Op::And, args)
    }
    fn ge(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Ge, vec![a, b])
    }
    fn lt(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Lt, vec![a, b])
    }
    fn eq(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Eq, vec![a, b])
    }
    fn ite(ctx: &mut Ctx, c: TermId, t: TermId, e: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Ite, vec![c, t, e])
    }
    fn add(ctx: &mut Ctx, a: TermId, b: TermId) -> Result<TermId, TypeError> {
        ctx.app(Op::Add, vec![a, b])
    }
    fn distinct(ctx: &mut Ctx, args: Vec<TermId>) -> Result<TermId, TypeError> {
        ctx.app(Op::Distinct, args)
    }

    // Cost matrix: cost[worker][task]
    let cost: [[i64; 4]; 4] = [
        [9, 2, 7, 8],
        [6, 4, 3, 7],
        [5, 8, 1, 8],
        [7, 6, 9, 4],
    ];
    // Latency matrix: latency[worker][task]
    let latency: [[i64; 4]; 4] = [
        [3, 7, 2, 5],
        [6, 1, 8, 3],
        [4, 9, 1, 7],
        [8, 2, 6, 2],
    ];
    // Risk matrix: risk[worker][task]
    let risk: [[i64; 4]; 4] = [
        [1, 5, 3, 4],
        [4, 2, 6, 1],
        [3, 7, 1, 5],
        [6, 3, 4, 2],
    ];

    let mut ctx = Ctx::new();

    // x[i] = task assigned to worker i, in [0, 4).
    let x: Vec<TermId> = (0..4)
        .map(|i| ctx.var(format!("x{i}"), Sort::Int))
        .collect();
    let zero = ctx.int_lit(0);
    let four = ctx.int_lit(4);

    let mut assertions = Vec::new();

    // Domain: 0 <= x[i] < 4
    for &xi in &x {
        let lo = ge(&mut ctx, xi, zero)?;
        let hi = lt(&mut ctx, xi, four)?;
        assertions.push(and(&mut ctx, vec![lo, hi])?);
    }

    // Each worker gets a distinct task.
    assertions.push(distinct(&mut ctx, x.clone())?);

    // Build objective expressions using nested ite chains.
    // For worker i, objective_value = ite(x[i]=0, m[i][0], ite(x[i]=1, m[i][1], ...))
    fn objective_for_worker(
        ctx: &mut Ctx,
        xi: TermId,
        row: &[i64; 4],
    ) -> Result<TermId, TypeError> {
        // Build right-to-left: start with the last value as the else branch.
        let mut expr = ctx.int_lit(row[3]);
        for j in (0..3).rev() {
            let task_j = ctx.int_lit(j as i64);
            let val_j = ctx.int_lit(row[j]);
            let cond = eq(ctx, xi, task_j)?;
            expr = ite(ctx, cond, val_j, expr)?;
        }
        Ok(expr)
    }

    // total_cost = sum of cost[i][x[i]]
    let mut total_cost = ctx.int_lit(0);
    for i in 0..4 {
        let worker_cost = objective_for_worker(&mut ctx, x[i], &cost[i])?;
        total_cost = add(&mut ctx, total_cost, worker_cost)?;
    }

    // max_latency = max of latency[i][x[i]], built as nested ite(a >= b, a, b)
    let mut worker_latencies = Vec::new();
    for i in 0..4 {
        worker_latencies.push(objective_for_worker(&mut ctx, x[i], &latency[i])?);
    }
    let mut max_latency = worker_latencies[0];
    for &lat in &worker_latencies[1..] {
        let cond = ge(&mut ctx, max_latency, lat)?;
        max_latency = ite(&mut ctx, cond, max_latency, lat)?;
    }

    // total_risk = sum of risk[i][x[i]]
    let mut total_risk = ctx.int_lit(0);
    for i in 0..4 {
        let worker_risk = objective_for_worker(&mut ctx, x[i], &risk[i])?;
        total_risk = add(&mut ctx, total_risk, worker_risk)?;
    }

    // Introduce named variables for objective values so we can read them from the model.
    let obj_cost = ctx.var("obj_cost", Sort::Int);
    let obj_latency = ctx.var("obj_latency", Sort::Int);
    let obj_risk = ctx.var("obj_risk", Sort::Int);

    assertions.push(eq(&mut ctx, obj_cost, total_cost)?);
    assertions.push(eq(&mut ctx, obj_latency, max_latency)?);
    assertions.push(eq(&mut ctx, obj_risk, total_risk)?);

    // Variables to project: assignments + objectives.
    let mut vars: Vec<TermId> = x.clone();
    vars.extend([obj_cost, obj_latency, obj_risk]);

    // All three objectives are minimized.
    let mut frontier = ParetoFrontier::new(vec![
        Direction::Minimize,
        Direction::Minimize,
        Direction::Minimize,
    ])
    .with_labels(vec![
        "cost".into(),
        "latency".into(),
        "risk".into(),
    ]);

    let mut enumerated = 0usize;
    loop {
        let r = smtkit::z3::solve_projected(&ctx, &assertions, &vars)?;
        match r.status {
            smtkit::SolveStatus::Sat => {
                enumerated += 1;
                let m = r.model.expect("sat implies model");

                let get_int = |name: &str| -> i64 {
                    match m.get(&name.into()).expect("projected var present") {
                        smtkit::z3::ModelValue::Int(n) => *n,
                        other => panic!("expected Int, got {other:?}"),
                    }
                };

                let c = get_int("obj_cost");
                let l = get_int("obj_latency");
                let ri = get_int("obj_risk");
                let assign: Vec<i64> = (0..4).map(|i| get_int(&format!("x{i}"))).collect();

                let added = frontier.push(
                    vec![c as f64, l as f64, ri as f64],
                    assign.clone(),
                );

                if added {
                    println!(
                        "  non-dominated: x={assign:?}  cost={c} latency={l} risk={ri}"
                    );
                }

                // Block this assignment from appearing again.
                let mut assigns = Vec::new();
                for &v in &x {
                    let sym = match ctx.kind_of(v) {
                        smtkit::core::TermKind::Var { sym, .. } => sym.clone(),
                        _ => continue,
                    };
                    let mv = m.get(&sym).expect("projected var present");
                    let lit = match *mv {
                        smtkit::z3::ModelValue::Int(n) => ctx.int_lit(n),
                        smtkit::z3::ModelValue::Bool(b) => ctx.bool_lit(b),
                        smtkit::z3::ModelValue::BitVec { value, width } => {
                            ctx.bv_lit(value, width)
                        }
                    };
                    assigns.push((v, lit));
                }
                let block = ctx.block_model(&assigns)?;
                assertions.push(block);
            }
            smtkit::SolveStatus::Unsat => break,
            smtkit::SolveStatus::Unknown => {
                eprintln!("solver returned unknown after {enumerated} solutions");
                break;
            }
        }
    }

    println!("\nenumerated {enumerated} feasible assignments");
    println!(
        "pareto frontier: {} non-dominated points\n",
        frontier.len()
    );

    let labels = frontier.labels();
    for (i, pt) in frontier.points().iter().enumerate() {
        println!(
            "  [{i}] {}={:.0}  {}={:.0}  {}={:.0}  assignment={:?}",
            labels[0], pt.values[0],
            labels[1], pt.values[1],
            labels[2], pt.values[2],
            pt.data,
        );
    }

    Ok(())
}

#[cfg(not(feature = "z3-inproc"))]
fn main() {
    eprintln!(
        "Enable this example with: cargo run -p smtkit --example pareto_frontier \
         --features z3-inproc-gh-release"
    );
}
