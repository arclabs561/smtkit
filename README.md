# smtkit

[![crates.io](https://img.shields.io/crates/v/smtkit.svg)](https://crates.io/crates/smtkit)
[![docs.rs](https://docs.rs/smtkit/badge.svg)](https://docs.rs/smtkit)

SMT solver toolkit.

## Install

```toml
[dependencies]
smtkit = "0.2"
```

The default build only generates SMT-LIB2 scripts. Solver features:

| Feature | What it enables |
|---|---|
| `z3-bin` | Run an external `z3` binary on `PATH` |
| `z3-inproc` | In-process Z3 through Rust bindings (may need system Z3 headers) |
| `z3-inproc-bundled` | `z3-inproc`, building Z3 from source |
| `z3-inproc-gh-release` | `z3-inproc`, downloading a prebuilt Z3 at build time |
| `z3-auto` | Alias for `z3-inproc-gh-release` |

## Library example: build an SMT-LIB2 script (no solver needed)

```rust
use smtkit::smt2::{t, Script, Sort, Var};

let x = Var::new("x", Sort::Int);
let mut s = Script::new();
s.set_logic("QF_LIA");
s.declare_const(&x);
s.assert(t::lt(x.sym(), t::int_lit(-3)));
s.check_sat();

let text = s.to_string();
assert!(text.contains("(declare-const x Int)"));
assert!(text.contains("(assert (< x (- 3)))"));
```

## Quickstart: probe solver capabilities

`smtkit-ci` is not published to crates.io. With a solver on `PATH` (for
example `z3`), run it from a clone of this repository:

```bash
cargo install --path crates/smtkit-ci --bin smtkit-ci --force
smtkit-ci probe
```

Or without installing:

```bash
cargo run -p smtkit-ci -- probe
```

It prints the solver command line it used and a best-effort capability matrix.
With Z3 5.3.2:

```text
{
  "available": true,
  "caps": {
    "check_sat_assuming": true,
    "get_model": true,
    "get_proof": true,
    "get_unsat_core": true,
    "named_assertions_in_core": true
  },
  ...
  "used": "z3 -in -smt2"
}
```

`get_proof` is solver-dependent and mainly a debugging hook.

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

To run the demo and capture a bounded result, including a proof preview:

```bash
smtkit-ci probe --capture-demo --demo-kind unsat-proof --demo-proof-max-chars 12000
```

## Crates

- `smtkit`: the crate you depend on. Re-exports `smtkit-core` and
  `smtkit-smtlib`, plus `smtkit-z3` with `z3-inproc`.
- `smtkit-core`: typed, backend-agnostic constraint IR.
- `smtkit-smtlib`: SMT-LIB2 s-expressions, script building, and an incremental stdio session.
- `smtkit-z3`: in-process Z3 backend.
- `smtkit-ci`: CLI for CI and debugging (`probe`, `smoke`); not published.

## Solver proofs

Some solvers support `(get-proof)` behind `:produce-proofs`. `smtkit` exposes
this through the session API and the capability probe (`get_proof`) so you can
capture proof objects for debugging.

`smtkit` does not check proofs. Verifying solver proofs (for example, Alethe
proof checking) is a separate layer.

## Examples

[`examples/README.md`](examples/README.md) lists each example with its run
command, required feature, and sample output. Examples without a feature emit
SMT-LIB2 scripts and need no solver; `z3-bin` examples run a system `z3`;
`z3-inproc` examples run Z3 in process.

```bash
cargo run --release --example graph_coloring
```

```text
(set-logic QF_LIA)
(set-option :produce-models true)
(declare-const c0 Int)
(assert (and (>= c0 0) (< c0 3)))
...
(assert (distinct c0 c1 c2))
(check-sat)
(get-model)
```

## License

Licensed under either the [Apache License, Version 2.0](LICENSE-APACHE) or
the [MIT license](LICENSE-MIT), at your option.
