use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "smtkit-ci")]
#[command(about = "Small utilities for smtkit CI + debugging.")]
struct Args {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum DemoKind {
    /// SAT demo with `(get-value ...)`.
    Sat,
    /// UNSAT demo that requests an unsat core (requires `:produce-unsat-cores` support).
    UnsatCore,
    /// UNSAT demo that requests a proof object (requires `:produce-proofs` + `(get-proof)` support).
    UnsatProof,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Minimal link-check: ensure the facade crate is usable.
    Smoke,
    /// Probe solver availability + best-effort capability matrix.
    Probe {
        /// Optional explicit SMTKIT_SOLVER-style command line to probe (e.g. `z3 -in -smt2`).
        #[arg(long)]
        cmdline: Option<String>,
        /// If set, emit a tiny SMT2 script that can be run with `z3 -in -smt2`.
        #[arg(long)]
        emit_demo_smt2: bool,
        /// Which demo script to emit (default: sat).
        #[arg(long, value_enum, default_value_t = DemoKind::Sat)]
        demo_kind: DemoKind,
        /// Optional path to write the probe JSON output.
        #[arg(long)]
        output_json: Option<std::path::PathBuf>,
        /// Optional path to write the demo `.smt2` script.
        #[arg(long)]
        demo_smt2_out: Option<std::path::PathBuf>,
        /// If set, run the demo against the solver and capture a bounded result.
        ///
        /// This is intended for debugging (e.g. "does this solver return a proof here?").
        #[arg(long)]
        capture_demo: bool,
        /// Maximum proof characters to include in the captured demo result.
        #[arg(long, default_value_t = 12_000)]
        demo_proof_max_chars: usize,
        /// Optional path to write the captured proof object (full text) when available.
        #[arg(long)]
        demo_proof_out: Option<std::path::PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    match args.cmd {
        Command::Smoke => {
            let _ = std::any::type_name::<smtkit::core::Ctx>();
            println!("smtkit-ci: ok");
        }
        Command::Probe {
            cmdline,
            emit_demo_smt2,
            demo_kind,
            output_json,
            demo_smt2_out,
            capture_demo,
            demo_proof_max_chars,
            demo_proof_out,
        } => {
            let (mut sess, used, caps) = if let Some(cmdline) = cmdline {
                let mut sess = smtkit::session::SmtlibSession::spawn_cmdline(&cmdline)?;
                let used = cmdline;
                let caps = smtkit::session::probe_capabilities(&mut sess);
                (sess, used, caps)
            } else {
                smtkit::session::spawn_auto_with_caps()?
            };

            // Best-effort: gather a tiny bit of extra observability.
            let reason_unknown = sess.get_info(":reason-unknown").ok();

            // Optional: run the demo and capture a bounded result.
            let demo_result: Option<serde_json::Value> = if capture_demo {
                fn truncate_chars(s: &str, max: usize) -> String {
                    if max == 0 {
                        return String::new();
                    }
                    if s.chars().count() <= max {
                        return s.to_string();
                    }
                    let mut out: String = s.chars().take(max).collect();
                    out.push_str("…");
                    out
                }

                let _ = sess.set_logic("QF_LIA");
                let _ = sess.set_print_success(false);
                match demo_kind {
                    DemoKind::Sat => {
                        let _ = sess.set_produce_models(true);
                        let _ = sess.declare_const("x", &smtkit::smt2::Sort::Int.to_smt2());
                        let _ = sess.assert_sexp(&smtkit::smt2::t::eq(
                            smtkit::smt2::t::sym("x"),
                            smtkit::smt2::t::int_lit(0),
                        ));
                        let st = sess.check_sat().ok();
                        let vals = sess
                            .get_value_pairs(&[smtkit::smt2::t::sym("x")])
                            .ok()
                            .map(|pairs| format!("{pairs:?}"));
                        Some(serde_json::json!({
                            "kind": "sat",
                            "status": st.map(|s| format!("{s:?}")),
                            "values": vals,
                        }))
                    }
                    DemoKind::UnsatCore => {
                        let _ = sess.set_produce_unsat_cores(true);
                        let _ = sess.assert_sexp(&smtkit::smt2::t::app(
                            "!",
                            vec![
                                smtkit::sexp::Sexp::atom("false"),
                                smtkit::sexp::Sexp::atom(":named"),
                                smtkit::sexp::Sexp::atom("h0"),
                            ],
                        ));
                        let st = sess.check_sat().ok();
                        let core = sess.get_unsat_core().ok().map(|c| c.to_string());
                        Some(serde_json::json!({
                            "kind": "unsat_core",
                            "status": st.map(|s| format!("{s:?}")),
                            "core": core,
                        }))
                    }
                    DemoKind::UnsatProof => {
                        let _ = sess.set_produce_proofs(true);
                        let _ = sess.assert_sexp(&smtkit::sexp::Sexp::atom("false"));
                        let st = sess.check_sat().ok();
                        let proof = sess.get_proof().ok().map(|p| p.to_string());
                        let proof_chars = proof.as_ref().map(|s| s.chars().count());
                        if let (Some(p), Some(path)) = (proof.as_ref(), demo_proof_out.as_ref()) {
                            // Write the full object as text for debugging.
                            let _ = std::fs::write(path, p);
                        }
                        Some(serde_json::json!({
                            "kind": "unsat_proof",
                            "status": st.map(|s| format!("{s:?}")),
                            "proof_chars": proof_chars,
                            "proof_preview": proof.as_ref().map(|s| truncate_chars(s, demo_proof_max_chars)),
                            "proof_out": demo_proof_out.as_ref().map(|p| p.display().to_string()),
                        }))
                    }
                }
            } else {
                None
            };

            let _ = sess.exit();

            let out = serde_json::json!({
                "available": true,
                "used": used,
                "caps": {
                    "check_sat_assuming": caps.check_sat_assuming,
                    "get_model": caps.get_model,
                    "get_unsat_core": caps.get_unsat_core,
                    "get_proof": caps.get_proof,
                    "named_assertions_in_core": caps.named_assertions_in_core,
                },
                "reason_unknown_info": reason_unknown.map(|s| s.to_string()),
                "demo": demo_result.unwrap_or(serde_json::Value::Null),
            });
            let out_pretty = serde_json::to_string_pretty(&out)?;
            println!("{out_pretty}");
            if let Some(p) = output_json {
                std::fs::write(&p, out_pretty)?;
            }

            if emit_demo_smt2 {
                let demo = match demo_kind {
                    DemoKind::Sat => {
                        // A tiny LIA script that is SAT with x=0.
                        r#"(set-logic QF_LIA)
(set-option :produce-models true)
(set-option :print-success false)
(declare-const x Int)
(assert (= x 0))
(check-sat)
(get-value (x))
"#
                    }
                    DemoKind::UnsatCore => {
                        // A tiny UNSAT script with a named assertion so cores are meaningful.
                        r#"(set-logic QF_LIA)
(set-option :produce-unsat-cores true)
(set-option :print-success false)
(assert (! false :named h0))
(check-sat)
(get-unsat-core)
"#
                    }
                    DemoKind::UnsatProof => {
                        // A tiny UNSAT script that asks for a proof object (solver-dependent).
                        r#"(set-logic QF_LIA)
(set-option :produce-proofs true)
(set-option :print-success false)
(assert false)
(check-sat)
(get-proof)
"#
                    }
                };

                if let Some(p) = demo_smt2_out {
                    std::fs::write(&p, demo)?;
                } else {
                    eprintln!("\n--- demo.smt2 ---\n{demo}--- end ---");
                }
            }
        }
    }
    Ok(())
}
