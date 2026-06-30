# smtkit examples

Each example answers one question and is runnable from the repo root. Output
excerpts below are real, captured from a run. Examples marked `z3-bin` require a
system `z3` on `PATH`; examples marked `z3-inproc` require the in-process Z3
feature.

## SMT-LIB scripts

### `graph_coloring`: how do I encode a graph-coloring problem?

Builds a QF_LIA script for coloring a triangle with three colors. No solver is
required; the example emits the SMT-LIB2 script.

```bash
cargo run --release --example graph_coloring
```
```text
(set-logic QF_LIA)
(set-option :produce-models true)
(declare-const c0 Int)
(assert (and (>= c0 0) (< c0 3)))
(declare-const c1 Int)
(assert (and (>= c1 0) (< c1 3)))
(declare-const c2 Int)
(assert (and (>= c2 0) (< c2 3)))
(assert (distinct c0 c1 c2))
(check-sat)
(get-model)
```

### `maximize_red`: what does an optimization objective look like?

Emits an SMT optimization script that maximizes the number of red vertices while
respecting adjacency constraints.

```bash
cargo run --release --example maximize_red
```
```text
(set-logic QF_LIA)
(set-option :produce-models true)
(declare-const c0 Int)
(declare-const c1 Int)
(declare-const c2 Int)
(declare-const c3 Int)
(assert (distinct c0 c1))
(assert (distinct c1 c2))
(assert (distinct c2 c3))
(maximize (+ (ite (= c0 0) 1 0) (ite (= c1 0) 1 0) (ite (= c2 0) 1 0) (ite (= c3 0) 1 0)))
(check-sat)
(get-model)
```

### `ontology_consistency`: can I inspect an ontology consistency problem?

Encodes a small EL++-style contradiction. Without `z3-inproc`, it emits the
SMT-LIB2 script; with `z3-inproc`, it solves and reports the contradiction.

```bash
cargo run --release --example ontology_consistency
```
```text
Ontology consistency check
  26 axiom instances, 25 variables

Axioms:
  1. Mammal ⊑ Animal
  2. Bird ⊑ Animal
  3. Penguin ⊑ Bird
  4. Bird ⊑ Flyer
  5. Penguin ⊓ Flyer ⊑ ⊥
  ABox: Penguin(ind_0)

(z3-inproc feature not enabled -- emitting SMT-LIB2 script instead)
```

```bash
cargo run --release --features z3-inproc --example ontology_consistency
```
```text
Result: UNSAT (ontology has a contradiction)

Explanation: Penguin(0) implies Bird(0) [axiom 3],
which implies Flyer(0) [axiom 4], but axiom 5 says
no individual can be both a Penguin and a Flyer.
```

## Solver sessions

### `session_determinism_hooks`: is the solver session reproducible?

Runs a small SAT query through a solver session with deterministic options.
Needs `z3-bin` and a system `z3`.

```bash
cargo run --release --features z3-bin --example session_determinism_hooks
```
```text
using solver: z3 -in -smt2
status: Sat
model: [(Atom("x"), Atom("0"))]
```

### `enumerate_graph_coloring_session`: how do I enumerate all models?

Uses an incremental SMT-LIB session and blocking clauses to enumerate all valid
triangle colorings. Needs `z3-bin` and a system `z3`.

```bash
cargo run --release --features z3-bin --example enumerate_graph_coloring_session
```
```text
using solver: z3 -in -smt2
model #1: [(Atom("c0"), Atom("0")), (Atom("c1"), Atom("1")), (Atom("c2"), Atom("2"))]
model #2: [(Atom("c0"), Atom("2")), (Atom("c1"), Atom("0")), (Atom("c2"), Atom("1"))]
...
model #6: [(Atom("c0"), Atom("1")), (Atom("c1"), Atom("2")), (Atom("c2"), Atom("0"))]
total models: 6
```

### `enumerate_graph_coloring_inproc`: can I enumerate models without a child process?

Runs the same blocking-clause loop through the in-process Z3 backend. Needs the
`z3-inproc` feature.

```bash
cargo run --release --features z3-inproc --example enumerate_graph_coloring_inproc
```
```text
model #1: {Sym("c2"): Int(2), Sym("c0"): Int(0), Sym("c1"): Int(1)}
model #2: {Sym("c2"): Int(2), Sym("c0"): Int(1), Sym("c1"): Int(0)}
...
model #6: {Sym("c1"): Int(2), Sym("c2"): Int(0), Sym("c0"): Int(1)}
total models: 6
```

## Optimization

### `pareto_frontier`: how do I enumerate trade-offs?

Enumerates feasible assignments with the in-process Z3 backend and reports the
non-dominated points over cost, latency, and risk. Needs `z3-inproc`.

```bash
cargo run --release --features z3-inproc --example pareto_frontier
```
```text
  non-dominated: x=[0, 1, 2, 3]  cost=18 latency=3 risk=6
  non-dominated: x=[1, 0, 2, 3]  cost=13 latency=7 risk=12

enumerated 24 feasible assignments
pareto frontier: 2 non-dominated points
```

### `constrained_soft_path`: how do hard constraints and soft path marginals combine?

Uses `smtkit-core` for hard routing constraints and `structops` for soft
shortest-path marginals over the feasible graph.

```bash
cargo run --release --example constrained_soft_path
```
```text
=== Phase 1: Hard constraints (smtkit-core) ===

Constraints encode: capacity bounds on 7 edges, flow balance on 4 intermediate nodes,
  source-flow = 5, budget <= 15.

Edge 2->4 (cost=8, cap=3) is infeasible: any path through it costs >= 16 > 15.

=== Phase 2: Soft shortest-path marginals (structops) ===

Feasible source-to-sink paths (0 -> 5):
  0->1->3->5  cost = 3+2+4 = 9   <-- shortest
  0->2->3->5  cost = 5+6+4 = 15
  (0->2->4->5 is pruned: edge 2->4 infeasible)

    edge \ gamma         0.1           1          10
                  ----------  ----------  ----------
      0->1 (c=3)    1.000000    0.997527    0.645656
      0->2 (c=5)    0.000000    0.002473    0.354344
      3->5 (c=4)    1.000000    1.000000    1.000000
```
