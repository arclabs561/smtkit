//! Constrained soft shortest path: smtkit-core constraints + structops soft optimization.
//!
//! Demonstrates a two-phase approach to constrained routing:
//!
//! 1. **Hard constraints** (smtkit-core): express edge-capacity bounds, flow-balance
//!    equations, and a budget constraint as typed SMT terms. No solver is invoked --
//!    the constraint IR is used purely for specification and inspection.
//!
//! 2. **Soft optimization** (structops): given the feasible edge set (edges that
//!    satisfy the hard constraints), compute soft shortest-path marginals at varying
//!    temperatures. Marginals show how "important" each edge is across the Gibbs
//!    distribution over all source-to-sink paths.
//!
//! The graph models a 6-node supply-chain network:
//!
//! ```text
//!   0 --[3]--> 1 --[2]--> 3 --[4]--> 5
//!   |          |                      ^
//!   +--[5]--> 2 --[1]--> 4 --[3]-----+
//!              |          ^
//!              +---[6]----+  (infeasible: exceeds capacity)
//! ```
//!
//! Edge 2->4 via the high-cost route is pruned by the capacity constraint,
//! leaving a reduced feasible graph for soft optimization.

use smtkit_core::{Ctx, Op, Sort, TermId};
use structops::soft_shortest_path::{soft_shortest_path_edge_marginals, Edge};

/// All candidate edges in the supply-chain network before constraint filtering.
/// Each tuple: (from, to, cost, capacity).
const CANDIDATE_EDGES: &[(usize, usize, f64, i64)] = &[
    (0, 1, 3.0, 20), // warehouse -> hub-A
    (0, 2, 5.0, 15), // warehouse -> hub-B
    (1, 3, 2.0, 10), // hub-A -> dist-center
    (2, 3, 6.0, 5),  // hub-B -> dist-center (limited capacity)
    (2, 4, 8.0, 3),  // hub-B -> relay (very limited -- will be pruned)
    (3, 5, 4.0, 12), // dist-center -> customer
    (4, 5, 3.0, 10), // relay -> customer
];

/// Required flow through the network.
const REQUIRED_FLOW: i64 = 5;

/// Total budget ceiling for the selected route.
const BUDGET: i64 = 15;

// ---------------------------------------------------------------------------
// Phase 1: express hard constraints in smtkit-core
// ---------------------------------------------------------------------------

/// Build the constraint IR for the supply-chain network.
///
/// Returns `(ctx, constraint_term, flow_var_ids, edge_labels)` where:
/// - `ctx` owns the term arena
/// - `constraint_term` is the top-level conjunction
/// - `flow_var_ids` maps edge index to its flow variable TermId
/// - `edge_labels` are human-readable names
fn build_constraints() -> (Ctx, TermId, Vec<TermId>, Vec<String>) {
    let mut ctx = Ctx::new();
    let mut conjuncts: Vec<TermId> = Vec::new();
    let mut flow_vars: Vec<TermId> = Vec::new();
    let mut edge_labels: Vec<String> = Vec::new();

    // Declare a flow variable per candidate edge and assert capacity bounds.
    for (i, &(from, to, _cost, cap)) in CANDIDATE_EDGES.iter().enumerate() {
        let name = format!("flow_{from}_{to}");
        edge_labels.push(format!("{from}->{to}"));
        let fv = ctx.var(name.as_str(), Sort::Int);
        flow_vars.push(fv);

        // 0 <= flow_i
        let zero = ctx.int_lit(0);
        let lb = ctx.app(Op::Le, vec![zero, fv]).expect("type-safe");
        conjuncts.push(lb);

        // flow_i <= capacity_i
        let cap_lit = ctx.int_lit(cap);
        let ub = ctx.app(Op::Le, vec![fv, cap_lit]).expect("type-safe");
        conjuncts.push(ub);

        let _ = i; // used for indexing only
    }

    // Flow-balance constraints for intermediate nodes (1, 2, 3, 4).
    // For each intermediate node: sum(incoming flow) == sum(outgoing flow).
    let intermediate_nodes = [1usize, 2, 3, 4];
    for &node in &intermediate_nodes {
        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();
        for (i, &(from, to, _, _)) in CANDIDATE_EDGES.iter().enumerate() {
            if to == node {
                incoming.push(flow_vars[i]);
            }
            if from == node {
                outgoing.push(flow_vars[i]);
            }
        }

        // Build sum terms (Add requires >= 2 args; if only 1, it is the sum itself).
        let in_sum = if incoming.len() >= 2 {
            ctx.app(Op::Add, incoming).expect("type-safe")
        } else if incoming.len() == 1 {
            incoming[0]
        } else {
            ctx.int_lit(0)
        };

        let out_sum = if outgoing.len() >= 2 {
            ctx.app(Op::Add, outgoing).expect("type-safe")
        } else if outgoing.len() == 1 {
            outgoing[0]
        } else {
            ctx.int_lit(0)
        };

        let balance = ctx.app(Op::Eq, vec![in_sum, out_sum]).expect("type-safe");
        conjuncts.push(balance);
    }

    // Source produces exactly REQUIRED_FLOW.
    let mut source_out = Vec::new();
    for (i, &(from, _, _, _)) in CANDIDATE_EDGES.iter().enumerate() {
        if from == 0 {
            source_out.push(flow_vars[i]);
        }
    }
    let source_sum = ctx.app(Op::Add, source_out).expect("type-safe");
    let req = ctx.int_lit(REQUIRED_FLOW);
    let source_eq = ctx.app(Op::Eq, vec![source_sum, req]).expect("type-safe");
    conjuncts.push(source_eq);

    // Budget constraint: sum(flow_i * cost_i) <= BUDGET.
    // Since smtkit-core has no Mul operator, we model this as an auxiliary variable
    // per edge: cost_i_total, and assert cost_i_total == flow_i * cost_i is
    // approximated by the constraint: cost_i_total <= capacity_i * unit_cost_i.
    // For the example, we use a simpler framing: assert that the total cost of
    // a single-unit path (the routing cost) is bounded.
    //
    // We express: for any selected edge, its unit cost contributes to the route.
    // Budget applies to the cheapest single route: we assert the existence of a
    // path whose total unit cost is within BUDGET.
    let budget_lit = ctx.int_lit(BUDGET);
    let mut cost_terms = Vec::new();
    for (i, &(_, _, cost, _)) in CANDIDATE_EDGES.iter().enumerate() {
        // ite(flow_i > 0, cost_i, 0) -- cost contributes only if edge is used.
        let zero = ctx.int_lit(0);
        let cost_lit = ctx.int_lit(cost as i64);
        let flow_pos = ctx
            .app(Op::Lt, vec![zero, flow_vars[i]])
            .expect("type-safe");
        let edge_cost = ctx
            .app(Op::Ite, vec![flow_pos, cost_lit, zero])
            .expect("type-safe");
        cost_terms.push(edge_cost);
    }
    let total_cost = ctx.app(Op::Add, cost_terms).expect("type-safe");
    let budget_cstr = ctx
        .app(Op::Le, vec![total_cost, budget_lit])
        .expect("type-safe");
    conjuncts.push(budget_cstr);

    // Top-level conjunction.
    let top = ctx.app(Op::And, conjuncts).expect("type-safe");

    (ctx, top, flow_vars, edge_labels)
}

// ---------------------------------------------------------------------------
// Phase 2: feasible-edge extraction + soft optimization
// ---------------------------------------------------------------------------

/// Given the constraints, determine feasible edges manually.
///
/// In a full pipeline a solver would produce the feasible set. Here we reason:
/// - Edge 2->4 has capacity 3 but cost 8. Using it forces total cost >= 5+8+3 = 16 > BUDGET.
///   So it is infeasible under the budget constraint.
/// - All other edges admit at least one path within budget.
fn feasible_edges() -> Vec<Edge> {
    CANDIDATE_EDGES
        .iter()
        .filter(|&&(from, to, cost, cap)| {
            // Prune edge 2->4: capacity too low and cost too high for any
            // budget-feasible path through it.
            let dominated = from == 2 && to == 4;
            let _ = (cost, cap); // used in reasoning above
            !dominated
        })
        .map(|&(from, to, cost, _cap)| Edge { from, to, cost })
        .collect()
}

fn main() {
    // -- Phase 1: build and inspect constraints --
    let (ctx, top, _flow_vars, edge_labels) = build_constraints();

    println!("=== Phase 1: Hard constraints (smtkit-core) ===");
    println!();
    println!("Candidate edges:");
    for (i, &(from, to, cost, cap)) in CANDIDATE_EDGES.iter().enumerate() {
        println!(
            "  [{i}] {from}->{to}  cost={cost:.0}  capacity={cap}  label={}",
            edge_labels[i]
        );
    }
    println!();

    let vars = ctx.free_vars(top);
    println!(
        "Constraint variables ({} total): {}",
        vars.len(),
        vars.iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();
    println!(
        "Constraints encode: capacity bounds on {} edges, flow balance on 4 intermediate nodes,",
        CANDIDATE_EDGES.len()
    );
    println!("  source-flow = {REQUIRED_FLOW}, budget <= {BUDGET}.");
    println!();
    println!(
        "Edge 2->4 (cost=8, cap=3) is infeasible: any path through it costs >= 16 > {BUDGET}.",
    );
    println!();

    // -- Phase 2: soft optimization on feasible subgraph --
    let edges = feasible_edges();
    let n = 6; // nodes 0..5

    println!("=== Phase 2: Soft shortest-path marginals (structops) ===");
    println!();
    println!(
        "Feasible edges ({} of {} candidates):",
        edges.len(),
        CANDIDATE_EDGES.len()
    );
    for (i, e) in edges.iter().enumerate() {
        println!("  [{i}] {}->{} cost={:.1}", e.from, e.to, e.cost);
    }
    println!();

    // Enumerate paths for reference.
    println!("Feasible source-to-sink paths (0 -> 5):");
    println!("  0->1->3->5  cost = 3+2+4 = 9   <-- shortest");
    println!("  0->2->3->5  cost = 5+6+4 = 15");
    println!("  (0->2->4->5 is pruned: edge 2->4 infeasible)");
    println!();

    // Sweep gamma: small = concentrating, large = spreading.
    let gammas = [0.1, 1.0, 10.0];

    // Header.
    print!("{:>16}", "edge \\ gamma");
    for &g in &gammas {
        print!("  {:>10}", format!("{g}"));
    }
    println!();
    print!("{:>16}", "");
    for _ in &gammas {
        print!("  ----------");
    }
    println!();

    // Compute marginals at each temperature.
    let results: Vec<(f64, Vec<f64>)> = gammas
        .iter()
        .map(|&g| {
            soft_shortest_path_edge_marginals(n, &edges, g)
                .expect("computation should succeed on feasible graph")
        })
        .collect();

    // Print per-edge marginals.
    for (i, e) in edges.iter().enumerate() {
        let label = format!("{}->{} (c={:.0})", e.from, e.to, e.cost);
        print!("{:>16}", label);
        for (_value, marginals) in &results {
            print!("  {:>10.6}", marginals[i]);
        }
        println!();
    }
    println!();

    // Print soft value (smoothed shortest-path length).
    print!("{:>16}", "soft value");
    for (value, _) in &results {
        print!("  {:>10.4}", value);
    }
    println!();
    println!();

    // Interpretation.
    println!("Interpretation:");
    println!(
        "  gamma=0.1:  marginals concentrate on the shortest feasible path (0->1->3->5, cost=9)."
    );
    println!(
        "              Edges 0->1, 1->3, 3->5 each get marginal ~1.0; the 0->2->3->5 path gets ~0."
    );
    println!("  gamma=1.0:  some mass shifts to the longer path (0->2->3->5, cost=15).");
    println!("  gamma=10.0: marginals spread toward uniform over both paths.");
    println!();
    println!(
        "The hard constraints (expressed in smtkit-core) determine *which* edges are feasible."
    );
    println!("The soft optimization (structops) determines *how important* each feasible edge is");
    println!("under the Gibbs distribution over paths, parameterized by temperature gamma.");
}
