//! Named script fields remain data even when they contain command-shaped text.
use smtkit::smt2::Script;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn descriptive_names_and_ordinary_multiline_diagnostics() {
    let mut script = Script::new();
    script.set_logic("descriptive logic (v2)");
    script.comment("first line\nsecond line\rthird line\r\nfourth line");
    assert_eq!(
        script.to_string(),
        "(set-logic |descriptive logic (v2)|)\n;first line\n;second line\r;third line\r;\n;fourth line\n"
    );
}

const HOSTILE_LOGIC: &str = "QF_LIA) (assert false) ;";
const INVALID_NAMES: &[&str] = &[
    "QF_LIA) (assert false) ;|",
    "bad\\name",
    "bad\0name",
    "bad\u{7f}name",
];

#[test]
fn term_symbol_names_match_declared_symbols() {
    use smtkit::smt2::{t, Sort, Var};
    for (name, expected) in [
        ("ordinary", "ordinary"),
        ("two words", "|two words|"),
        ("(not x)", "|(not x)|"),
        ("α", "|α|"),
        ("|prequoted name|", "|prequoted name|"),
    ] {
        assert_eq!(t::sym(name).to_string(), expected);
        assert_eq!(t::sym(name), Var::new(name, Sort::Bool).sym());
    }
}

#[test]
fn term_symbol_rejection_precedes_script_mutation() {
    use smtkit::smt2::t;
    let mut script = Script::new();
    script.set_logic("QF_UF");
    for name in ["bad|name", "bad\\name", "bad\0name", "bad\u{7f}name"] {
        let before = script.to_string();
        assert!(catch_unwind(AssertUnwindSafe(|| script.assert(t::sym(name)))).is_err());
        assert_eq!(script.to_string(), before);
    }
}

#[test]
fn logic_is_one_symbol_and_invalid_names_leave_script_unchanged() {
    let mut script = Script::new();
    script.set_logic(HOSTILE_LOGIC);
    assert_eq!(
        script.to_string(),
        "(set-logic |QF_LIA) (assert false) ;|)\n"
    );
    for logic in ["QF_LIA", "|QF_LIA|"] {
        let mut script = Script::new();
        script.set_logic(logic);
        assert_eq!(script.to_string(), format!("(set-logic {logic})\n"));
    }
    for name in INVALID_NAMES {
        let before = script.to_string();
        assert!(catch_unwind(AssertUnwindSafe(|| script.set_logic(*name))).is_err());
        assert_eq!(script.to_string(), before);
    }
}

#[test]
fn every_comment_line_stays_a_comment() {
    for (input, expected) in [
        ("", ";\n"),
        ("hello", ";hello\n"),
        ("hello\n(assert false)", ";hello\n;(assert false)\n"),
        ("hello\r(assert false)", ";hello\r;(assert false)\n"),
        ("hello\r\n(assert false)", ";hello\r;\n;(assert false)\n"),
        ("hello\n", ";hello\n;\n"),
        ("hello\r", ";hello\r;\n"),
        ("diagnostic λ\r\n", ";diagnostic λ\r;\n;\n"),
    ] {
        let mut script = Script::new();
        script.comment(input);
        assert_eq!(script.to_string(), expected);
    }
}

#[cfg(feature = "z3-bin")]
mod solver {
    use super::*;
    use smtkit::session::{SmtlibSession, Status};
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn term_symbol_references_keep_declared_name_meaning() {
        use smtkit::smt2::{t, Sort, Var};
        for name in ["ordinary", "two words", "(not x)", "α", "|prequoted name|"] {
            let mut script = Script::new();
            script.set_logic("QF_UF");
            let x = Var::new("x", Sort::Bool);
            let named = Var::new(name, Sort::Bool);
            script.declare_const(&x);
            script.declare_const(&named);
            script.assert(x.sym());
            script.assert(t::sym(name));
            script.check_sat();
            let mut child = Command::new("z3")
                .args(["-in", "-smt2"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("z3-bin boundary tests require z3 on PATH");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(script.to_string().as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(output.status.success(), "name={name:?}: {stdout}");
            assert_eq!(stdout.trim(), "sat", "name={name:?}");
        }
    }

    fn assert_script_sat(script: &Script) {
        let mut child = Command::new("z3")
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("z3-bin boundary tests require z3 on PATH");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        // An unknown quoted logic may be rejected. It must never execute the
        // assertion inside its name, even if the solver continues after an error.
        assert!(stdout.lines().any(|line| line == "sat"), "{stdout}");
        assert!(!stdout.lines().any(|line| line == "unsat"), "{stdout}");
    }

    #[test]
    fn script_data_cannot_change_solver_satisfiability() {
        for logic in ["QF_LIA", "|QF_LIA|", HOSTILE_LOGIC] {
            let mut script = Script::new();
            script.set_logic(logic);
            script.check_sat();
            assert_script_sat(&script);
        }
        for separator in ["\n", "\r", "\r\n"] {
            let mut script = Script::new();
            script.set_logic("QF_LIA");
            script.comment(format!("diagnostic{separator}(assert false)"));
            script.check_sat();
            assert_script_sat(&script);
        }
    }

    #[test]
    fn session_logic_cannot_add_assertions_and_rejects_before_io() {
        for logic in ["QF_LIA", "|QF_LIA|", HOSTILE_LOGIC] {
            let mut session = SmtlibSession::spawn("z3", &["-in", "-smt2"]).unwrap();
            let result = session.set_logic(logic);
            if logic != HOSTILE_LOGIC {
                assert!(result.is_ok(), "{result:?}");
            }
            assert_eq!(session.check_sat().unwrap(), Status::Sat);
            session.exit().unwrap();
        }
        for name in INVALID_NAMES {
            let mut session = SmtlibSession::spawn("z3", &["-in", "-smt2"]).unwrap();
            assert!(catch_unwind(AssertUnwindSafe(|| session.set_logic(name))).is_err());
            // The first valid set-logic must still succeed, without an earlier
            // command or error response contaminating the session.
            session.set_logic("QF_LIA").unwrap();
            assert_eq!(session.check_sat().unwrap(), Status::Sat);
            session.exit().unwrap();
        }
    }
}
