# smtkit

[![crates.io](https://img.shields.io/crates/v/smtkit.svg)](https://crates.io/crates/smtkit)
[![docs.rs](https://docs.rs/smtkit/badge.svg)](https://docs.rs/smtkit)

SMT solver toolkit.

## Quickstart: probe solver capabilities

If you have a solver on PATH (e.g. `z3`), run:

```bash
cargo install --path crates/smtkit-ci --bin smtkit-ci --force
smtkit-ci probe
```

If you prefer to run from this repo without installing:

```bash
cargo run -p smtkit-ci -- probe
```

This prints JSON like:

- which command line was used (e.g. `z3 -in -smt2`)
- a **capability matrix** (best-effort):
  - `check_sat_assuming`
  - `get_model`
  - `get_unsat_core`
  - `get_proof` (solver-dependent; primarily a debugging hook)
  - whether `:named` assertions appear in cores

You can also generate repro scripts:

```bash
smtkit-ci probe --emit-demo-smt2 --demo-kind sat
smtkit-ci probe --emit-demo-smt2 --demo-kind unsat-core
smtkit-ci probe --emit-demo-smt2 --demo-kind unsat-proof
```

Or write artifacts to disk:

```bash
smtkit-ci probe --output-json /tmp/smtkit_probe.json \
  --emit-demo-smt2 --demo-kind unsat-core --demo-smt2-out /tmp/demo.smt2
```

If you want `smtkit-ci` to **run** the demo and capture a bounded result (including a proof preview),
use:

```bash
smtkit-ci probe --capture-demo --demo-kind unsat-proof --demo-proof-max-chars 12000
```

## Replayable Solver Runs

SMT integrations need stable scripts, recorded solver options, and reproducible
solver runs. `smtkit` emits SMT-LIB scripts and records probe output so failures
can be replayed.

## Crates

- **`smtkit`**: facade crate you depend on (re-exports the rest).
- **`smtkit-core`**: typed, backend-agnostic constraint IR.
- **`smtkit-smtlib`**: SMT-LIB2 s-expressions, script building, and an incremental stdio session.
- **`smtkit-z3`**: optional in-process Z3 backend (feature-gated).
- **`smtkit-ci`**: small CLI for CI + debugging (`probe`, `smoke`).

## Notes on solver “proofs”

Some solvers support `(get-proof)` behind `:produce-proofs`. `smtkit` exposes this via the session API
and the capability probe (`get_proof`) so you can **capture** proof objects for debugging/provenance.

This is **not** a proof checker: verification of solver proofs (e.g. Alethe proof checking) is a separate layer.

## Examples

See [`examples/README.md`](examples/README.md) for the full gallery: each
example states the question it answers, the run command, feature requirements,
and real sample output.

Runnable examples live in [`examples/`](examples/). Unmarked examples emit
SMT-LIB2 scripts and need no solver installed (smtkit's core posture); `z3-bin`
examples shell out to a system `z3`; `z3-inproc` examples drive Z3 in process.

- `graph_coloring` encodes a small graph-coloring instance, the textbook constraint problem behind register allocation and exam scheduling.
- `maximize_red` is a small optimization-modulo-theories instance: maximize an objective subject to constraints.
- `ontology_consistency` checks EL++ ontology consistency as SMT, the satisfiability core of description-logic knowledge-base validation.
- `enumerate_graph_coloring_session` enumerates every valid coloring of a triangle via an incremental SMT-LIB session, the blocking-clause pattern for counting or sampling all solutions rather than just one. (needs `z3-bin`)
- `session_determinism_hooks` is a smoke test for the determinism hooks that make solver runs reproducible across machines. (needs `z3-bin`)
- `enumerate_graph_coloring_inproc` is the enumeration above run through the in-process backend. (needs `z3-inproc`)
- `pareto_frontier` enumerates the Pareto frontier of a multi-objective problem, the shape of trade-off analysis in resource allocation. (needs `z3-inproc`)
- `constrained_soft_path` solves a shortest path with hard SMT constraints plus soft preferences, the mix routing and planning problems usually need. (needs `z3-inproc`)

## Versioning + pinning policy (recommended)

- **Apps should pin**: if you build a tool like `proofpatch`, prefer `smtkit = "0.x.y"` (crates.io) and commit `Cargo.lock`.
- **Libraries can float**: if you’re publishing a library, prefer semver ranges and do not commit `Cargo.lock`.
- **Pre-release testing**: for unreleased changes, temporarily pin a git tag (e.g. `v0.1.1`) or a git rev, then switch back to crates.io on release.

## License

Licensed under either the [Apache License, Version 2.0](LICENSE-APACHE) or
the [MIT license](LICENSE-MIT), at your option.
