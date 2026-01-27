# smtkit

[![CI](https://github.com/arclabs561/smtkit/actions/workflows/ci.yml/badge.svg)](https://github.com/arclabs561/smtkit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/smtkit.svg)](https://crates.io/crates/smtkit)
[![docs.rs](https://docs.rs/smtkit/badge.svg)](https://docs.rs/smtkit)

`smtkit` is a small Rust toolkit for reproducible SMT workflows.

### What it gives you

- **A canonical SMT-LIB2 surface**: build scripts deterministically, dump them, and rerun them.
- **A stdio session API**: run solvers as processes and capture status/model/core/proof (best-effort).
- **An optional in-process Z3 adapter** (feature-gated).

### Why it exists

SMT integrations tend to go wrong in the same ways:

- Tooling silently depends on solver quirks (Z3 vs cvc5 behavior).
- Results are hard to reproduce (no stable script, no determinism hooks recorded).
- Debugging “why UNSAT?” is opaque (no unsat core, no minimal fragment, no provenance).

`smtkit` is built around the opposite posture: emit the script, run the solver, keep the evidence.

### Crates

- **`smtkit`**: facade crate you depend on (re-exports the rest).
- **`smtkit-core`**: typed, backend-agnostic constraint IR.
- **`smtkit-smtlib`**: SMT-LIB2 s-expressions, script building, and an incremental stdio session.
- **`smtkit-z3`**: optional in-process Z3 backend (feature-gated).
- **`smtkit-ci`**: repo-local CLI for CI + debugging.

### Quickstart: probe solver capabilities

From this repo:

```bash
cargo run -p smtkit-ci -- probe
```

You can also generate repro scripts:

```bash
cargo run -p smtkit-ci -- probe --emit-demo-smt2 --demo-kind sat
cargo run -p smtkit-ci -- probe --emit-demo-smt2 --demo-kind unsat-core
cargo run -p smtkit-ci -- probe --emit-demo-smt2 --demo-kind unsat-proof
```

Or write artifacts to disk:

```bash
cargo run -p smtkit-ci -- probe --output-json /tmp/smtkit_probe.json \
  --emit-demo-smt2 --demo-kind unsat-core --demo-smt2-out /tmp/demo.smt2
```

If you want `smtkit-ci` to **run** the demo and capture a bounded result (including a proof preview),
use:

```bash
cargo run -p smtkit-ci -- probe --capture-demo --demo-kind unsat-proof --demo-proof-max-chars 12000
```

### Notes on solver “proofs”

Some solvers support `(get-proof)` behind `:produce-proofs`. `smtkit` exposes this via the session API
and the capability probe (`get_proof`) so you can **capture** proof objects for debugging/provenance.

This is **not** a proof checker: verification of solver proofs (e.g. Alethe proof checking) is a separate layer.

