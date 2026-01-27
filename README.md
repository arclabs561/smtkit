# smtkit

[![CI](https://github.com/arclabs561/smtkit/actions/workflows/ci.yml/badge.svg)](https://github.com/arclabs561/smtkit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/smtkit.svg)](https://crates.io/crates/smtkit)
[![docs.rs](https://docs.rs/smtkit/badge.svg)](https://docs.rs/smtkit)

`smtkit` is a small Rust toolkit for **reproducible SMT workflows**:

- **Build** constraints (typed IR in `smtkit-core`).
- **Emit** SMT-LIB2 (`smtkit-smtlib`).
- **Run** an external solver over stdio (optional).
- **Capture artifacts** (capability matrix, SMT2 scripts, models/cores/proofs) so solver-driven behavior is debuggable and comparable across solvers.

## Why it exists

SMT integrations tend to go wrong in the same ways:

- Tooling silently depends on solver quirks (Z3 vs cvc5 behavior).
- Results are hard to reproduce (no stable script, no determinism hooks recorded).
- Debugging “why UNSAT?” is opaque (no unsat core, no minimal fragment, no provenance).

`smtkit` is built around the opposite posture: **emit the script, run the solver, and keep the evidence**.

## Crates

- **`smtkit`**: facade crate you depend on (re-exports the rest).
- **`smtkit-core`**: typed, backend-agnostic constraint IR.
- **`smtkit-smtlib`**: SMT-LIB2 s-expressions, script building, and an incremental stdio session.
- **`smtkit-z3`**: optional in-process Z3 backend (feature-gated).
- **`smtkit-ci`**: small CLI for CI + debugging (`probe`, `smoke`).

## Quickstart: probe solver capabilities

If you have a solver on PATH (e.g. `z3`), run:

```bash
smtkit-ci probe
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

## Notes on solver “proofs”

Some solvers support `(get-proof)` behind `:produce-proofs`. `smtkit` exposes this via the session API
and the capability probe (`get_proof`) so you can **capture** proof objects for debugging/provenance.

This is **not** a proof checker: verification of solver proofs (e.g. Alethe proof checking) is a separate layer.

## Versioning + pinning policy (recommended)

- **Apps should pin**: if you build a tool like `proofpatch`, prefer `smtkit = "0.x.y"` (crates.io) and commit `Cargo.lock`.
- **Libraries can float**: if you’re publishing a library, prefer semver ranges and do not commit `Cargo.lock`.
- **Pre-release testing**: for unreleased changes, temporarily pin a git tag (e.g. `v0.1.1`) or a git rev, then switch back to crates.io on release.

