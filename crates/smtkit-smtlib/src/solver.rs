//! External solver runners (optional, backend-specific).
//!
//! Core idea: we emit SMT-LIB2, then run a solver binary (e.g. `z3 -in`).

#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    #[error("failed to spawn solver process")]
    Spawn(#[source] std::io::Error),
    #[error("solver returned non-zero exit code: {code}\nstdout:\n{stdout}\nstderr:\n{stderr}")]
    NonZero {
        code: i32,
        stdout: String,
        stderr: String,
    },
    #[error("failed to write SMT2 to solver stdin")]
    Stdin(#[source] std::io::Error),
    #[error("failed to read solver output")]
    Output(#[source] std::io::Error),
}

/// Solver status for a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Sat,
    Unsat,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseSolverOutputError {
    #[error("missing solver status line in stdout")]
    MissingStatus,
    #[error("unknown solver status: {0}")]
    UnknownStatus(String),
    #[error("failed to parse model s-expression")]
    Model(#[source] crate::sexp::ParseError),
}

/// Parsed Z3 output: status + optional model.
#[derive(Clone, Debug)]
pub struct Z3Output {
    pub status: Status,
    pub model: Option<crate::sexp::Sexp>,
    pub raw_stdout: String,
    pub raw_stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanState {
    Normal,
    InString,
    InBarSymbol,
}

/// Best-effort extraction of the first complete SMT-LIB response from `text`.
///
/// This is intended to tolerate solver noise such as:
/// - multiple top-level s-expressions
/// - string literals (`"..."`) where parentheses should not count
/// - bar-quoted symbols (`|...|`) where parentheses should not count
///
/// If the first non-whitespace token is not `(`, we return the first whitespace-delimited token.
fn extract_first_response(text: &str) -> Option<String> {
    let mut it = text.chars().peekable();
    // Skip leading whitespace and line comments.
    loop {
        match it.peek().copied() {
            None => return None,
            Some(ch) if ch.is_whitespace() => {
                it.next();
                continue;
            }
            Some(';') => {
                // SMT-LIB line comment.
                it.next();
                while let Some(c) = it.next() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            _ => break,
        }
    }

    // Atom response (e.g. `unsupported`).
    if it.peek().copied() != Some('(') {
        let mut out = String::new();
        while let Some(ch) = it.peek().copied() {
            if ch.is_whitespace() || ch == ';' {
                break;
            }
            out.push(ch);
            it.next();
        }
        if out.trim().is_empty() {
            None
        } else {
            Some(out)
        }
    } else {
        let mut out = String::new();
        let mut depth: i64 = 0;
        let mut state = ScanState::Normal;
        while let Some(ch) = it.next() {
            match state {
                ScanState::Normal => {
                    // SMT-LIB line comments: ignore until newline (but keep the newline for structure).
                    if ch == ';' {
                        while let Some(c) = it.next() {
                            if c == '\n' {
                                out.push('\n');
                                break;
                            }
                        }
                        continue;
                    }
                    if ch == '"' {
                        state = ScanState::InString;
                        out.push(ch);
                        continue;
                    }
                    if ch == '|' {
                        state = ScanState::InBarSymbol;
                        out.push(ch);
                        continue;
                    }
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                    }
                    out.push(ch);
                    if depth == 0 {
                        break;
                    }
                }
                ScanState::InString => {
                    out.push(ch);
                    if ch == '"' {
                        // SMT-LIB escapes quotes as `""`.
                        if it.peek().copied() == Some('"') {
                            out.push('"');
                            it.next();
                            continue;
                        }
                        state = ScanState::Normal;
                    }
                }
                ScanState::InBarSymbol => {
                    out.push(ch);
                    if ch == '|' {
                        // SMT-LIB allows `||` to embed a literal `|`.
                        if it.peek().copied() == Some('|') {
                            out.push('|');
                            it.next();
                            continue;
                        }
                        state = ScanState::Normal;
                    }
                }
            }
        }
        let t = out.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }
}

fn is_errorish_line(line: &str) -> bool {
    let t = line.trim();
    t == "success"
        || t == "unsupported"
        || t.starts_with("(error")
        || t.starts_with("(warning")
        || t.starts_with("(unsupported")
}

/// Parse Z3 stdout/stderr into status + optional model.
///
/// Contract: the first non-empty line is the status (`sat|unsat|unknown`).
/// Any remaining non-empty output is treated as a single s-expression model.
pub fn parse_z3_output(stdout: &str, stderr: &str) -> Result<Z3Output, ParseSolverOutputError> {
    // Note: we avoid treating `success` or `(error ...)` chatter as part of the model. Z3 usually
    // doesn't print those for `check-sat`/`get-model`, but other tool wrappers occasionally do.
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !is_errorish_line(l));
    let status_line = lines.next().ok_or(ParseSolverOutputError::MissingStatus)?;
    let status = match status_line {
        "sat" => Status::Sat,
        "unsat" => Status::Unsat,
        "unknown" => Status::Unknown,
        other => return Err(ParseSolverOutputError::UnknownStatus(other.to_string())),
    };

    let rest: String = lines.collect::<Vec<_>>().join("\n");
    let model_txt = extract_first_response(&rest).unwrap_or(rest);
    let model = if model_txt.trim().is_empty() || is_errorish_line(&model_txt) {
        None
    } else {
        Some(
            crate::sexp::parse_one(&model_txt).map_err(ParseSolverOutputError::Model)?,
        )
    };

    Ok(Z3Output {
        status,
        model,
        raw_stdout: stdout.to_string(),
        raw_stderr: stderr.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_z3_output_ignores_success_and_warning_lines() {
        let stdout = "success\nsat\n\n(warning \"noise\")\n\n(model (define-fun x () Int 5))\n";
        let out = parse_z3_output(stdout, "").expect("parse");
        assert_eq!(out.status, Status::Sat);
        assert!(out.model.is_some(), "expected model");
    }

    #[test]
    fn parse_z3_output_accepts_multiple_top_level_sexps_by_taking_first() {
        let stdout = "sat\n(model (define-fun x () Int 5))\n(extra (stuff))\n";
        let out = parse_z3_output(stdout, "").expect("parse");
        assert_eq!(out.status, Status::Sat);
        assert_eq!(
            out.model.unwrap().to_string(),
            "(model (define-fun x () Int 5))"
        );
    }

    #[test]
    fn extract_first_response_handles_strings_and_bar_symbols() {
        let txt = r#"(a "x) y" |p(q)|) (b c)"#;
        let first = extract_first_response(txt).expect("first");
        assert_eq!(first, r#"(a "x) y" |p(q)|)"#);
    }
}

/// Run `z3` (must be on PATH) with SMT-LIB2 provided on stdin.
///
/// Evidence contract: caller can log `argv` + exit code + captured stdout/stderr.
#[cfg(feature = "z3-bin")]
pub fn run_z3_stdin(smt2: &str, extra_args: &[&str]) -> Result<(String, String), SolverError> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("z3");
    cmd.arg("-in").arg("-smt2");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(SolverError::Spawn)?;
    {
        use std::io::Write;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| SolverError::Stdin(std::io::Error::other("missing stdin")))?;
        stdin
            .write_all(smt2.as_bytes())
            .map_err(SolverError::Stdin)?;
    }

    let out = child.wait_with_output().map_err(SolverError::Output)?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(SolverError::NonZero {
            code: out.status.code().unwrap_or(-1),
            stdout,
            stderr,
        });
    }
    Ok((stdout, stderr))
}
