//! Incremental SMT-LIB process session (solver-agnostic).
//!
//! This is intentionally minimal and built on stdio pipes:
//! - Spawn a solver that supports `-in` + SMT-LIB2 (e.g. Z3, cvc5)
//! - Send commands on stdin
//! - Read responses from stdout
//!
//! Higher-level backends can add richer parsing and capabilities as needed.

use std::io::ErrorKind;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::sexp::Sexp;

/// A small, best-effort capability snapshot for an SMT-LIB session.
///
/// This exists because solver feature support differs in practice, and callers frequently want
/// a structured “what is likely to work?” answer instead of hard-coding solver names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Supports `(check-sat-assuming ...)`.
    pub check_sat_assuming: bool,
    /// Supports `(get-model)` after `sat` when model production is enabled.
    pub get_model: bool,
    /// Supports `(get-unsat-core)` after `unsat` when core production is enabled.
    pub get_unsat_core: bool,
    /// Supports `(get-proof)` after `unsat` when proof production is enabled.
    pub get_proof: bool,
    /// Whether `:named` assertions are reflected in the returned core (useful for explanations).
    pub named_assertions_in_core: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Sat,
    Unsat,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    #[error("failed to spawn solver process: {cmd}")]
    Spawn {
        cmd: String,
        #[source]
        source: std::io::Error,
    },
    #[error("missing solver stdin")]
    MissingStdin,
    #[error("missing solver stdout")]
    MissingStdout,
    #[error("failed to write to solver stdin")]
    Stdin(#[source] std::io::Error),
    #[error("failed to read solver stdout")]
    Stdout(#[source] std::io::Error),
    #[error("unexpected EOF while reading solver output")]
    Eof,
    #[error("solver does not support the requested SMT-LIB command")]
    Unsupported,
    #[error("solver rejected an SMT-LIB command: {message}")]
    Solver { message: String },
    #[error("solver did not respond to {operation} within {timeout:?}")]
    Timeout {
        operation: &'static str,
        timeout: Duration,
    },
    #[error("solver response exceeded the configured {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("solver session is unusable after an earlier protocol or timeout error")]
    Poisoned,
    #[error("unknown solver status line: {0}")]
    UnknownStatus(String),
    #[error("failed to parse s-expression response")]
    Parse(#[source] crate::sexp::ParseError),
    #[error("unexpected (get-value ...) response shape")]
    BadGetValue,
    #[error("failed to wait for solver process to exit")]
    Wait(#[source] std::io::Error),
    #[error("no solver found on PATH (tried: {tried:?})")]
    NoSolverFound { tried: Vec<String> },
}

/// A running solver process speaking SMT-LIB over stdio.
pub struct SmtlibSession {
    child: Child,
    stdin: Option<mpsc::Sender<StdinRequest>>,
    stdin_thread: Option<JoinHandle<()>>,
    stdout: mpsc::Receiver<StdoutEvent>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    wall_timeout: Duration,
    max_response_bytes: Arc<AtomicUsize>,
    poisoned: bool,
    next_sync_id: u64,
}

const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
const STDERR_TAIL_LIMIT: usize = 16 * 1024;
/// Large enough for ordinary models and captured proof objects while bounding
/// one untrusted solver response.
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

struct StdinRequest {
    bytes: Vec<u8>,
    completed: mpsc::SyncSender<Result<(), std::io::Error>>,
}

enum StdoutEvent {
    Line(String),
    Io(std::io::Error),
    ResponseTooLarge { limit: usize },
}

/// Read exactly one line without retaining more bytes than the response limit.
///
/// `BufRead::read_line` allocates until it encounters a newline, so it cannot
/// enforce this boundary when a solver emits one giant unterminated line.
fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_response_bytes: &AtomicUsize,
) -> Result<Option<String>, StdoutEvent> {
    let mut bytes = Vec::new();
    loop {
        let (take, has_newline) = {
            let available = reader.fill_buf().map_err(StdoutEvent::Io)?;
            if available.is_empty() {
                if bytes.is_empty() {
                    return Ok(None);
                }
                let text = String::from_utf8(bytes).map_err(|error| {
                    StdoutEvent::Io(std::io::Error::new(ErrorKind::InvalidData, error))
                })?;
                return Ok(Some(text));
            }
            match available.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (available.len(), false),
            }
        };

        let limit = max_response_bytes.load(Ordering::Relaxed);
        if bytes.len().saturating_add(take) > limit {
            return Err(StdoutEvent::ResponseTooLarge { limit });
        }

        {
            let available = reader.fill_buf().map_err(StdoutEvent::Io)?;
            bytes.extend_from_slice(&available[..take]);
        }
        reader.consume(take);
        if has_newline {
            let text = String::from_utf8(bytes).map_err(|error| {
                StdoutEvent::Io(std::io::Error::new(ErrorKind::InvalidData, error))
            })?;
            return Ok(Some(text));
        }
    }
}

impl SmtlibSession {
    /// Spawn a solver process.
    ///
    /// Example (Z3): `spawn("z3", &["-in", "-smt2"])`
    pub fn spawn(cmd: &str, args: &[&str]) -> Result<Self, SessionError> {
        let mut c = Command::new(cmd);
        c.args(args);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let display_cmd = std::iter::once(cmd.to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .join(" ");

        let mut child = c.spawn().map_err(|e| SessionError::Spawn {
            cmd: display_cmd,
            source: e,
        })?;
        let mut stdin = child.stdin.take().ok_or(SessionError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(SessionError::MissingStdout)?;
        let stderr = child.stderr.take();
        let (stdin_tx, stdin_rx) = mpsc::channel::<StdinRequest>();
        let stdin_thread = std::thread::spawn(move || {
            while let Ok(request) = stdin_rx.recv() {
                let result = stdin.write_all(&request.bytes).and_then(|()| stdin.flush());
                let failed = result.is_err();
                let _ = request.completed.send(result);
                if failed {
                    break;
                }
            }
        });
        let max_response_bytes = Arc::new(AtomicUsize::new(DEFAULT_MAX_RESPONSE_BYTES));
        let reader_limit = Arc::clone(&max_response_bytes);
        // One queued line means the reader cannot accumulate an unbounded
        // aggregate behind a slow response parser.
        let (stdout_tx, stdout_rx) = mpsc::sync_channel(1);
        let stdout_thread = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_bounded_line(&mut reader, &reader_limit) {
                    Ok(None) => break,
                    Ok(Some(line)) => {
                        if stdout_tx.send(StdoutEvent::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(event) => {
                        let _ = stdout_tx.send(event);
                        break;
                    }
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_thread = stderr.map(|mut stderr| {
            let tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                let mut chunk = [0_u8; 4096];
                while let Ok(n) = stderr.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut captured) = tail.lock() {
                        captured.extend_from_slice(&chunk[..n]);
                        if captured.len() > STDERR_TAIL_LIMIT {
                            let excess = captured.len() - STDERR_TAIL_LIMIT;
                            captured.drain(..excess);
                        }
                    }
                }
            })
        });
        Ok(Self {
            child,
            stdin: Some(stdin_tx),
            stdin_thread: Some(stdin_thread),
            stdout: stdout_rx,
            stdout_thread: Some(stdout_thread),
            stderr_thread,
            stderr_tail,
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            max_response_bytes,
            poisoned: false,
            next_sync_id: 0,
        })
    }

    /// Set the maximum wall-clock time spent writing a command or waiting for
    /// any single solver response.
    ///
    /// A timeout terminates and reaps the child process; the session cannot be reused.
    pub fn set_wall_timeout(&mut self, timeout: Duration) {
        self.wall_timeout = timeout;
    }

    /// Set the maximum number of bytes in one solver response.
    ///
    /// This bounds the bytes retained for each stdout line and the cumulative text
    /// of a balanced SMT-LIB response. It is not a bound on all memory held by
    /// the session: a reader's partial line, one queued line, and the response
    /// accumulator can briefly coexist. A zero limit rejects every non-empty
    /// solver response. Breaching the limit terminates and reaps the direct
    /// solver child; the session cannot be reused.
    pub fn set_max_response_bytes(&mut self, limit: usize) {
        self.max_response_bytes.store(limit, Ordering::Relaxed);
    }

    /// Return the bounded tail of solver diagnostics captured from stderr.
    pub fn stderr_tail(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|tail| String::from_utf8_lossy(&tail).into_owned())
            .unwrap_or_default()
    }

    /// Spawn a solver from a shell-like command line.
    ///
    /// This is useful for env overrides like:
    /// - `SMTKIT_SOLVER="z3 -in -smt2"`
    /// - `SMTKIT_SOLVER="cvc5 --lang smt2 --incremental"`
    ///
    /// Quoting is handled via `shell-words` (simple POSIX-ish rules).
    pub fn spawn_cmdline(cmdline: &str) -> Result<Self, SessionError> {
        let parts = shell_words::split(cmdline).map_err(|e| SessionError::Spawn {
            cmd: cmdline.to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
        })?;
        let (cmd, args) = parts.split_first().ok_or_else(|| SessionError::Spawn {
            cmd: cmdline.to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command line"),
        })?;
        let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Self::spawn(cmd, &args)
    }

    /// Send raw SMT-LIB text (must include trailing newline if needed).
    pub fn send_raw(&mut self, s: &str) -> Result<(), SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let request = StdinRequest {
            bytes: s.as_bytes().to_vec(),
            completed: completed_tx,
        };
        if self
            .stdin
            .as_ref()
            .ok_or(SessionError::Poisoned)?
            .send(request)
            .is_err()
        {
            return Err(SessionError::Stdin(std::io::Error::new(
                ErrorKind::BrokenPipe,
                "solver stdin writer stopped",
            )));
        }
        match completed_rx.recv_timeout(self.wall_timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(SessionError::Stdin(error)),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(SessionError::Stdin(
                std::io::Error::new(ErrorKind::BrokenPipe, "solver stdin writer stopped"),
            )),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.poisoned = true;
                self.terminate_and_reap();
                Err(SessionError::Timeout {
                    operation: "solver stdin write",
                    timeout: self.wall_timeout,
                })
            }
        }
    }

    /// Read the next non-empty line from stdout (trimmed).
    fn read_line_until(
        &mut self,
        operation: &'static str,
        started: Instant,
    ) -> Result<String, SessionError> {
        let elapsed = started.elapsed();
        if elapsed >= self.wall_timeout {
            self.poisoned = true;
            self.terminate_and_reap();
            return Err(SessionError::Timeout {
                operation,
                timeout: self.wall_timeout,
            });
        }
        let remaining = self.wall_timeout - elapsed;
        match self.stdout.recv_timeout(remaining) {
            Ok(StdoutEvent::Line(line)) => {
                // The reader may have accepted this line before a concurrent
                // caller lowered the shared limit. Recheck at the consumer
                // boundary before exposing it to a response parser.
                let limit = self.max_response_bytes.load(Ordering::Relaxed);
                if line.len() > limit {
                    self.fail_response_limit(limit)
                } else {
                    Ok(line)
                }
            }
            Ok(StdoutEvent::Io(error)) => Err(SessionError::Stdout(error)),
            Ok(StdoutEvent::ResponseTooLarge { limit }) => self.fail_response_limit(limit),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(SessionError::Eof),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.poisoned = true;
                self.terminate_and_reap();
                Err(SessionError::Timeout {
                    operation,
                    timeout: self.wall_timeout,
                })
            }
        }
    }

    fn response_error(line: &str) -> Option<SessionError> {
        let trimmed = line.trim();
        if trimmed == "unsupported" || trimmed.starts_with("(unsupported") {
            Some(SessionError::Unsupported)
        } else if trimmed.starts_with("(error") {
            Some(SessionError::Solver {
                message: trimmed.to_string(),
            })
        } else {
            None
        }
    }

    fn fail_protocol<T>(&mut self, error: SessionError) -> Result<T, SessionError> {
        self.poisoned = true;
        self.terminate_and_reap();
        Err(error)
    }

    fn fail_response_limit<T>(&mut self, limit: usize) -> Result<T, SessionError> {
        self.poisoned = true;
        self.terminate_and_reap();
        Err(SessionError::ResponseTooLarge { limit })
    }

    /// Send a command that normally has no specific response, then use SMT-LIB
    /// `echo` as an ordering fence. This attributes `unsupported`/`(error ...)`
    /// to the command that caused it instead of leaving the response to poison a
    /// later `check-sat` or `get-*` call.
    fn send_checked(&mut self, command: &str, operation: &'static str) -> Result<(), SessionError> {
        let token = format!("smtkit-sync-{}", self.next_sync_id);
        self.next_sync_id = self.next_sync_id.wrapping_add(1);
        self.send_raw(&format!("{command}(echo \"{token}\")\n"))?;

        let started = Instant::now();
        let expected = format!("\"{token}\"");
        let mut command_error = None;
        loop {
            let line = self.read_line_until(operation, started)?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed == "success" {
                continue;
            }
            if trimmed == expected || trimmed == token {
                return command_error.map_or(Ok(()), Err);
            }
            if let Some(error) = Self::response_error(trimmed) {
                command_error.get_or_insert(error);
                continue;
            }
            return self.fail_protocol(SessionError::Solver {
                message: format!("unexpected response while synchronizing {operation}: {trimmed}"),
            });
        }
    }

    fn read_nonempty_line(&mut self, operation: &'static str) -> Result<String, SessionError> {
        let started = Instant::now();
        loop {
            let line = self.read_line_until(operation, started)?;
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            // Some solvers print "success" for commands when :print-success is enabled.
            // We do not enable it, but we defensively ignore it.
            if t == "success" {
                continue;
            }
            if let Some(error) = Self::response_error(t) {
                return Err(error);
            }
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }

    /// Read a single s-expression response from stdout by balancing parentheses.
    fn read_sexp_balanced(&mut self, operation: &'static str) -> Result<String, SessionError> {
        let started_at = Instant::now();
        let mut buf = String::new();
        let mut depth: i64 = 0;
        let mut started = false;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum ScanState {
            Normal,
            InString,
            InBarSymbol,
        }

        fn scan_line_update_depth(
            line: &str,
            depth: &mut i64,
            started: &mut bool,
            st: &mut ScanState,
        ) {
            let bytes = line.as_bytes();
            let mut i: usize = 0;
            while i < bytes.len() {
                let b = bytes[i];
                match *st {
                    ScanState::Normal => {
                        // SMT-LIB line comments: ignore rest of line.
                        if b == b';' {
                            break;
                        }
                        if b == b'"' {
                            *st = ScanState::InString;
                            i += 1;
                            continue;
                        }
                        if b == b'|' {
                            *st = ScanState::InBarSymbol;
                            i += 1;
                            continue;
                        }
                        if b == b'(' {
                            *depth += 1;
                            *started = true;
                        } else if b == b')' {
                            *depth -= 1;
                        }
                        i += 1;
                    }
                    ScanState::InString => {
                        if b == b'\\' {
                            // Best-effort: consume escape + following byte if present.
                            i += 1;
                            if i < bytes.len() {
                                i += 1;
                            }
                            continue;
                        }
                        if b == b'"' {
                            // SMT-LIB escapes `"` as `""`.
                            if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                                i += 2;
                                continue;
                            }
                            *st = ScanState::Normal;
                            i += 1;
                            continue;
                        }
                        i += 1;
                    }
                    ScanState::InBarSymbol => {
                        if b == b'|' {
                            // SMT-LIB allows `||` to embed a literal `|` inside a bar-quoted symbol.
                            if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                                i += 2;
                                continue;
                            }
                            *st = ScanState::Normal;
                            i += 1;
                            continue;
                        }
                        i += 1;
                    }
                }
            }
        }

        let mut state = ScanState::Normal;
        loop {
            let line = self.read_line_until(operation, started_at)?;

            if !started {
                let t = line.trim();
                if t.is_empty() || t == "success" {
                    continue;
                }
                if let Some(error) = Self::response_error(t) {
                    return Err(error);
                }
                // Some solvers respond to `(get-...)` with a single atom like `unsupported`.
                // Accept that as a complete response (so callers can parse it as an atom).
                if !t.starts_with('(') {
                    return Ok(format!("{t}\n"));
                }
            }

            let limit = self.max_response_bytes.load(Ordering::Relaxed);
            if buf.len().saturating_add(line.len()) > limit {
                return self.fail_response_limit(limit);
            }
            scan_line_update_depth(&line, &mut depth, &mut started, &mut state);
            buf.push_str(&line);

            if started && depth == 0 {
                return Ok(buf);
            }
        }
    }

    /// `(push 1)` (no response expected).
    pub fn push(&mut self) -> Result<(), SessionError> {
        self.send_checked("(push 1)\n", "push")
    }

    /// `(pop n)` (no response expected).
    pub fn pop(&mut self, n: u32) -> Result<(), SessionError> {
        self.send_checked(&format!("(pop {n})\n"), "pop")
    }

    /// `(set-logic LOGIC)` (no response expected).
    ///
    /// The logic name is rendered as one SMT-LIB symbol.
    ///
    /// # Panics
    ///
    /// Panics before writing to the solver if [`crate::emit::quote_symbol`] rejects the name.
    pub fn set_logic(&mut self, logic: &str) -> Result<(), SessionError> {
        let logic = crate::emit::quote_symbol(logic);
        self.send_checked(&format!("(set-logic {logic})\n"), "set-logic")
    }

    /// `(set-option KEY VALUE)` (no response expected).
    pub fn set_option(&mut self, key: &str, value: &Sexp) -> Result<(), SessionError> {
        self.send_checked(&format!("(set-option {key} {value})\n"), "set-option")
    }

    /// Best-effort: enable/disable model production.
    ///
    /// Many solvers (e.g. Z3) accept `(set-option :produce-models true|false)`.
    pub fn set_produce_models(&mut self, enabled: bool) -> Result<(), SessionError> {
        self.set_option(
            ":produce-models",
            &Sexp::atom(if enabled { "true" } else { "false" }),
        )
    }

    /// Best-effort: enable/disable unsat core production (solver-dependent).
    ///
    /// Notes:
    /// - Z3 supports `(set-option :produce-unsat-cores true|false)`.
    /// - Not all solvers support unsat cores.
    pub fn set_produce_unsat_cores(&mut self, enabled: bool) -> Result<(), SessionError> {
        self.set_option(
            ":produce-unsat-cores",
            &Sexp::atom(if enabled { "true" } else { "false" }),
        )
    }

    /// Best-effort: enable/disable proof production (solver-dependent).
    ///
    /// Notes:
    /// - Some solvers support `(set-option :produce-proofs true|false)` and `(get-proof)`.
    /// - Proof formats are solver-specific; treat this as a debugging/provenance hook.
    pub fn set_produce_proofs(&mut self, enabled: bool) -> Result<(), SessionError> {
        self.set_option(
            ":produce-proofs",
            &Sexp::atom(if enabled { "true" } else { "false" }),
        )
    }

    /// Best-effort: enable/disable printing `success` for commands.
    ///
    /// We already ignore `success` in the reader, but this can reduce chatter on stdout.
    pub fn set_print_success(&mut self, enabled: bool) -> Result<(), SessionError> {
        self.set_option(
            ":print-success",
            &Sexp::atom(if enabled { "true" } else { "false" }),
        )
    }

    /// Best-effort, solver-side timeout in milliseconds (solver-dependent).
    ///
    /// Notes:
    /// - Z3 supports `(set-option :timeout <ms>)`.
    /// - Not all solvers support `:timeout` or interpret it the same way.
    pub fn set_timeout_ms(&mut self, ms: u64) -> Result<(), SessionError> {
        self.set_option(":timeout", &Sexp::atom(ms.to_string()))
    }

    /// Best-effort, solver-side random seed (solver-dependent).
    ///
    /// Notes:
    /// - Z3 supports `(set-option :random-seed <n>)`.
    pub fn set_random_seed(&mut self, seed: u64) -> Result<(), SessionError> {
        self.set_option(":random-seed", &Sexp::atom(seed.to_string()))
    }

    /// `(declare-const NAME SORT)` (no response expected).
    ///
    /// # Panics
    ///
    /// Panics before sending a command if [`crate::emit::quote_symbol`] rejects
    /// the name.
    pub fn declare_const(&mut self, name: &str, sort: &Sexp) -> Result<(), SessionError> {
        let name = crate::emit::quote_symbol(name);
        self.send_checked(&format!("(declare-const {name} {sort})\n"), "declare-const")
    }

    /// `(assert TERM)` (no response expected).
    pub fn assert_sexp(&mut self, term: &Sexp) -> Result<(), SessionError> {
        self.send_checked(&format!("(assert {term})\n"), "assert")
    }

    /// Assert a typed IR term from `smtkit-core` by serializing it to SMT-LIB.
    pub fn assert_term(
        &mut self,
        ctx: &smtkit_core::Ctx,
        t: smtkit_core::TermId,
    ) -> Result<(), SessionError> {
        let sexp = crate::emit::term_to_sexp(ctx, t);
        self.assert_sexp(&sexp)
    }

    /// `(check-sat)` → status.
    pub fn check_sat(&mut self) -> Result<Status, SessionError> {
        self.send_raw("(check-sat)\n")?;
        let line = self.read_nonempty_line("check-sat")?;
        match line.as_str() {
            "sat" => Ok(Status::Sat),
            "unsat" => Ok(Status::Unsat),
            "unknown" => Ok(Status::Unknown),
            other => Err(SessionError::UnknownStatus(other.to_string())),
        }
    }

    /// `(check-sat-assuming (a1 a2 ...))` → status.
    ///
    /// This is solver-dependent but widely supported by incremental SMT solvers (e.g. Z3, cvc5).
    /// It provides a “temporary assertion” mechanism without needing `push/pop`.
    pub fn check_sat_assuming(&mut self, assumptions: &[Sexp]) -> Result<Status, SessionError> {
        let mut cmd = String::from("(check-sat-assuming (");
        for (i, a) in assumptions.iter().enumerate() {
            if i > 0 {
                cmd.push(' ');
            }
            cmd.push_str(&a.to_string());
        }
        cmd.push_str("))\n");
        self.send_raw(&cmd)?;
        let line = self.read_nonempty_line("check-sat-assuming")?;
        match line.as_str() {
            "sat" => Ok(Status::Sat),
            "unsat" => Ok(Status::Unsat),
            "unknown" => Ok(Status::Unknown),
            other => Err(SessionError::UnknownStatus(other.to_string())),
        }
    }

    /// `(get-unsat-core)` → parsed s-expression.
    ///
    /// This is meaningful only after an `unsat` result and when unsat-core production is enabled.
    pub fn get_unsat_core(&mut self) -> Result<Sexp, SessionError> {
        self.send_raw("(get-unsat-core)\n")?;
        let s = self.read_sexp_balanced("get-unsat-core")?;
        crate::sexp::parse_one(&s).map_err(SessionError::Parse)
    }

    /// `(get-info KEY)` → parsed s-expression.
    ///
    /// Useful for diagnosing `unknown` results. Common keys include:
    /// - `:reason-unknown`
    ///
    /// Note: solver support is not guaranteed; callers should treat errors as "info unavailable".
    pub fn get_info(&mut self, key: &str) -> Result<Sexp, SessionError> {
        self.send_raw(&format!("(get-info {key})\n"))?;
        let s = self.read_sexp_balanced("get-info")?;
        crate::sexp::parse_one(&s).map_err(SessionError::Parse)
    }

    /// `(get-model)` → parsed model s-expression.
    pub fn get_model(&mut self) -> Result<Sexp, SessionError> {
        self.send_raw("(get-model)\n")?;
        let s = self.read_sexp_balanced("get-model")?;
        crate::sexp::parse_one(&s).map_err(SessionError::Parse)
    }

    /// `(get-proof)` → parsed proof s-expression (solver-dependent).
    pub fn get_proof(&mut self) -> Result<Sexp, SessionError> {
        self.send_raw("(get-proof)\n")?;
        let s = self.read_sexp_balanced("get-proof")?;
        crate::sexp::parse_one(&s).map_err(SessionError::Parse)
    }

    /// `(get-value ( ... ))` → parsed s-expression.
    ///
    /// This is intentionally generic: callers can interpret the returned s-expression.
    pub fn get_value(&mut self, terms: &[Sexp]) -> Result<Sexp, SessionError> {
        // (get-value (t1 t2 ...))
        let mut cmd = String::from("(get-value (");
        for (i, t) in terms.iter().enumerate() {
            if i > 0 {
                cmd.push(' ');
            }
            cmd.push_str(&t.to_string());
        }
        cmd.push_str("))\n");
        self.send_raw(&cmd)?;
        let s = self.read_sexp_balanced("get-value")?;
        crate::sexp::parse_one(&s).map_err(SessionError::Parse)
    }

    /// `(get-value ...)` decoded as a list of `(term, value)` pairs.
    ///
    /// For example: `((x 5) (y 3))` → `[(x, 5), (y, 3)]`.
    pub fn get_value_pairs(&mut self, terms: &[Sexp]) -> Result<Vec<(Sexp, Sexp)>, SessionError> {
        let resp = self.get_value(terms)?;
        decode_get_value_pairs(resp)
    }

    /// Send `(exit)` and wait for process termination.
    ///
    /// This is best-effort; some solvers may exit immediately, others may flush output first.
    pub fn exit(mut self) -> Result<std::process::ExitStatus, SessionError> {
        let _ = self.send_raw("(exit)\n");
        self.stdin.take();
        let status = self.wait_for_exit("exit")?;
        self.join_readers();
        Ok(status)
    }

    /// Best-effort terminate the child process.
    pub fn kill(&mut self) {
        self.terminate_and_reap();
    }

    fn terminate_and_reap(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.join_readers();
    }

    fn wait_for_exit(
        &mut self,
        operation: &'static str,
    ) -> Result<std::process::ExitStatus, SessionError> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().map_err(SessionError::Wait)? {
                return Ok(status);
            }
            if started.elapsed() >= self.wall_timeout {
                self.poisoned = true;
                self.terminate_and_reap();
                return Err(SessionError::Timeout {
                    operation,
                    timeout: self.wall_timeout,
                });
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn join_readers(&mut self) {
        Self::join_if_finished(&mut self.stdin_thread);
        if let Some(thread) = self.stdout_thread.take() {
            while self.stdout.try_recv().is_ok() {}
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                // A descendant may have inherited the solver's stdout pipe.
                // Dropping the handle detaches the reader instead of making
                // session teardown wait for an unrelated process lifetime.
                self.stdout_thread = Some(thread);
            }
        }
        Self::join_if_finished(&mut self.stderr_thread);
    }

    fn join_if_finished(thread: &mut Option<JoinHandle<()>>) {
        if thread.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for SmtlibSession {
    fn drop(&mut self) {
        self.terminate_and_reap();
    }
}

fn is_spawn_not_found(e: &SessionError) -> bool {
    matches!(e, SessionError::Spawn { source, .. } if source.kind() == ErrorKind::NotFound)
}

/// Best-effort probe of solver capabilities for a live session.
///
/// The probe is designed to be low-impact:
/// - It attempts to isolate checks with `push/pop`.
/// - It uses tiny formulas (`true`, `false`) and avoids declarations.
/// - It treats any error as “capability not available”.
///
/// Note: solver behavior around unsupported options varies (some emit `(error ...)` on stdout).
/// This probe relies on the session reader’s filtering and should be treated as best-effort.
pub fn probe_capabilities(sess: &mut SmtlibSession) -> Capabilities {
    let mut caps = Capabilities::default();

    fn is_errorish(sexp: &Sexp) -> bool {
        match sexp {
            Sexp::Atom(a) => {
                let t = a.trim();
                t == "unsupported" || t == "unknown" || t.starts_with("error")
            }
            Sexp::List(items) => match items.first() {
                Some(Sexp::Atom(h)) => h == "error" || h == "unsupported" || h == "warning",
                _ => false,
            },
        }
    }

    // Capability-enabling options must precede the first check: cvc5 rejects
    // several of them after the solver has entered its initialized state.
    let _ = sess.set_print_success(false);
    let models_enabled = sess.set_produce_models(true).is_ok();
    let cores_enabled = sess.set_produce_unsat_cores(true).is_ok();
    let proofs_enabled = sess.set_produce_proofs(true).is_ok();

    // check-sat-assuming
    caps.check_sat_assuming = sess.check_sat_assuming(&[]).is_ok();

    // get-model
    if sess.push().is_ok() {
        let _ = sess.assert_sexp(&Sexp::atom("true"));
        if models_enabled && matches!(sess.check_sat(), Ok(Status::Sat)) {
            caps.get_model = sess.get_model().is_ok();
        }
        let _ = sess.pop(1);
    }

    // get-unsat-core (and whether :named shows up)
    if sess.push().is_ok() {
        let named_false = Sexp::list(vec![
            Sexp::atom("!"),
            Sexp::atom("false"),
            Sexp::atom(":named"),
            Sexp::atom("h0"),
        ]);
        let _ = sess.assert_sexp(&named_false);
        if cores_enabled && matches!(sess.check_sat(), Ok(Status::Unsat)) {
            if let Ok(core) = sess.get_unsat_core() {
                caps.get_unsat_core = true;
                if let Sexp::List(items) = core {
                    caps.named_assertions_in_core = items
                        .into_iter()
                        .any(|it| matches!(it, Sexp::Atom(a) if a == "h0"));
                }
            }
        }
        let _ = sess.pop(1);
    }

    // get-proof (solver-dependent)
    if sess.push().is_ok() {
        let _ = sess.assert_sexp(&Sexp::atom("false"));
        if proofs_enabled && matches!(sess.check_sat(), Ok(Status::Unsat)) {
            if let Ok(pf) = sess.get_proof() {
                // Some solvers respond with `(error ...)` or `unsupported`.
                if !is_errorish(&pf) {
                    caps.get_proof = true;
                }
            }
        }
        let _ = sess.pop(1);
    }

    caps
}

/// Spawn a solver session using `SMTKIT_SOLVER` if set, otherwise try a small set of known solvers.
///
/// Returns the session plus the command line that was used.
pub fn spawn_auto() -> Result<(SmtlibSession, String), SessionError> {
    if let Ok(cmdline) = std::env::var("SMTKIT_SOLVER") {
        let sess = SmtlibSession::spawn_cmdline(&cmdline)?;
        return Ok((sess, cmdline));
    }

    // Small, explicit candidate set (bounded + predictable).
    //
    // - Z3: reads from stdin with `-in` and uses SMT-LIB2 with `-smt2`
    // - cvc5: incremental + SMT-LIB2 mode
    let candidates = ["z3 -in -smt2", "cvc5 --lang smt2 --incremental"];

    let mut tried = Vec::new();
    for cmdline in candidates {
        tried.push(cmdline.to_string());
        match SmtlibSession::spawn_cmdline(cmdline) {
            Ok(sess) => return Ok((sess, cmdline.to_string())),
            Err(e) if is_spawn_not_found(&e) => continue,
            Err(e) => return Err(e),
        }
    }

    Err(SessionError::NoSolverFound { tried })
}

/// Like `spawn_auto`, but also returns a best-effort capability snapshot.
///
/// This is useful for callers that want to avoid solver-name conditionals and instead branch on
/// a structured “what is likely to work” answer.
pub fn spawn_auto_with_caps() -> Result<(SmtlibSession, String, Capabilities), SessionError> {
    let (mut sess, used) = spawn_auto()?;
    let caps = probe_capabilities(&mut sess);
    Ok((sess, used, caps))
}

/// Decode a `(get-value ...)` response into a list of `(term, value)` pairs.
pub fn decode_get_value_pairs(resp: Sexp) -> Result<Vec<(Sexp, Sexp)>, SessionError> {
    match resp {
        Sexp::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Sexp::List(kv) if kv.len() == 2 => out.push((kv[0].clone(), kv[1].clone())),
                    _ => return Err(SessionError::BadGetValue),
                }
            }
            Ok(out)
        }
        _ => Err(SessionError::BadGetValue),
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use std::time::Instant;

    fn shell_session(script: &str) -> SmtlibSession {
        SmtlibSession::spawn("sh", &["-c", script]).expect("spawn fake solver")
    }

    #[test]
    fn unsupported_is_typed_without_misaligning_next_status() {
        let mut session = shell_session(
            "n=0; while IFS= read -r line; do case \"$line\" in *check-sat*) \
             if [ $n -eq 0 ]; then printf 'unsupported\\n'; else printf 'sat\\n'; fi; \
             n=$((n + 1));; esac; done",
        );

        assert!(matches!(
            session.check_sat(),
            Err(SessionError::Unsupported)
        ));
        assert_eq!(session.check_sat().unwrap(), Status::Sat);
    }

    #[test]
    fn solver_error_is_typed_without_misaligning_next_status() {
        let mut session = shell_session(
            "n=0; while IFS= read -r line; do case \"$line\" in *check-sat*) \
             if [ $n -eq 0 ]; then printf '(error \"bad option\")\\n'; else printf 'sat\\n'; fi; \
             n=$((n + 1));; esac; done",
        );

        let error = session.check_sat().expect_err("solver error must surface");
        assert!(matches!(
            error,
            SessionError::Solver { ref message } if message.contains("bad option")
        ));
        assert_eq!(session.check_sat().unwrap(), Status::Sat);
    }

    #[test]
    fn checked_command_surfaces_unsupported_without_misaligning_next_query() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in \
             *set-option*) printf 'unsupported\\n' ;; \
             *smtkit-sync-0*) printf '\"smtkit-sync-0\"\\n' ;; \
             *check-sat*) printf 'sat\\n' ;; esac; done",
        );

        assert!(matches!(
            session.set_timeout_ms(1),
            Err(SessionError::Unsupported)
        ));
        assert_eq!(session.check_sat().unwrap(), Status::Sat);
    }

    #[test]
    fn stderr_is_drained_while_capture_stays_bounded() {
        let mut session = shell_session(
            "i=0; while [ $i -lt 20000 ]; do \
             printf '0123456789abcdef\\n' >&2; i=$((i + 1)); done; \
             while IFS= read -r line; do \
             case \"$line\" in *check-sat*) printf 'sat\\n' ;; esac; done",
        );
        session.set_wall_timeout(Duration::from_secs(5));

        assert_eq!(session.check_sat().unwrap(), Status::Sat);
        let diagnostics = session.stderr_tail();
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.len() <= STDERR_TAIL_LIMIT);
    }

    #[test]
    fn wall_timeout_terminates_and_reaps_solver() {
        let mut session = shell_session(
            "while IFS= read -r line; do \
             case \"$line\" in *check-sat*) exec sleep 10 ;; esac; done",
        );
        session.set_wall_timeout(Duration::from_millis(50));
        let started = Instant::now();

        assert!(matches!(
            session.check_sat(),
            Err(SessionError::Timeout {
                operation: "check-sat",
                ..
            })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(session.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn timeout_does_not_join_pipes_inherited_by_descendants() {
        let mut session = shell_session(
            "sleep 3 & while IFS= read -r line; do \
             case \"$line\" in *check-sat*) sleep 3 ;; esac; done",
        );
        session.set_wall_timeout(Duration::from_millis(50));
        let started = Instant::now();

        assert!(matches!(
            session.check_sat(),
            Err(SessionError::Timeout {
                operation: "check-sat",
                ..
            })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(session.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn drop_does_not_join_pipes_inherited_by_descendants() {
        let started = Instant::now();
        {
            let _session = shell_session("sleep 3 & while IFS= read -r line; do :; done");
        }
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn wall_timeout_includes_blocked_stdin_write() {
        let mut session = shell_session("sleep 3");
        session.set_wall_timeout(Duration::from_millis(50));
        let command = "x".repeat(8 * 1024 * 1024);
        let started = Instant::now();

        assert!(matches!(
            session.send_raw(&command),
            Err(SessionError::Timeout {
                operation: "solver stdin write",
                ..
            })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(session.send_raw("x"), Err(SessionError::Poisoned)));
    }

    #[test]
    fn checked_command_rejects_unexpected_echo_fence_response() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in \
             *smtkit-sync-0*) printf 'forged-fence\\n' ;; esac; done",
        );

        let error = session.push().expect_err("unexpected fence must fail");
        assert!(matches!(
            error,
            SessionError::Solver { ref message } if message.contains("forged-fence")
        ));
        assert!(matches!(session.check_sat(), Err(SessionError::Poisoned)));
    }

    #[test]
    fn exit_is_also_bounded_by_wall_timeout() {
        let mut session = shell_session(
            "while IFS= read -r line; do \
             case \"$line\" in *exit*) exec sleep 10 ;; esac; done",
        );
        session.set_wall_timeout(Duration::from_millis(50));
        let started = Instant::now();

        assert!(matches!(
            session.exit(),
            Err(SessionError::Timeout {
                operation: "exit",
                ..
            })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn wall_timeout_bounds_the_whole_response_not_each_line() {
        let mut session = shell_session(
            "while IFS= read -r line; do \
             case \"$line\" in *check-sat*) while :; do printf 'success\\n'; done ;; esac; done",
        );
        session.set_wall_timeout(Duration::from_millis(50));
        let started = Instant::now();

        assert!(matches!(
            session.check_sat(),
            Err(SessionError::Timeout {
                operation: "check-sat",
                ..
            })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn unterminated_line_one_byte_over_limit_is_terminal_before_newline() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in *check-sat*) \\
             i=0; while [ $i -lt 65 ]; do printf x; i=$((i + 1)); done; sleep 10 ;; esac; done",
        );
        session.set_max_response_bytes(64);
        session.set_wall_timeout(Duration::from_secs(1));
        let started = Instant::now();

        assert!(matches!(
            session.check_sat(),
            Err(SessionError::ResponseTooLarge { limit: 64 })
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(session.child.try_wait().unwrap().is_some());
        assert!(matches!(session.check_sat(), Err(SessionError::Poisoned)));
    }

    #[test]
    fn balanced_response_limit_counts_all_lines_and_newlines() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in *get-info*) \\
             printf '(a\\n'; printf 'bbb\\n'; printf 'x\\n' ;; esac; done",
        );
        session.set_max_response_bytes(8);

        assert!(matches!(
            session.get_info(":name"),
            Err(SessionError::ResponseTooLarge { limit: 8 })
        ));
        assert!(session.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn balanced_response_exactly_at_limit_is_accepted() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in *get-info*) printf '(a)\\n' ;; esac; done",
        );
        session.set_max_response_bytes(4);

        assert_eq!(session.get_info(":name").unwrap().to_string(), "(a)");
    }

    #[test]
    fn lowered_limit_is_checked_again_when_a_queued_line_is_consumed() {
        let mut session = shell_session(
            "while IFS= read -r line; do case \"$line\" in *check-sat*) printf 'sat\\n' ;; esac; done",
        );
        session.set_max_response_bytes(4);
        session.send_raw("(check-sat)\n").unwrap();
        session.set_max_response_bytes(3);

        assert!(matches!(
            session.read_nonempty_line("test response limit"),
            Err(SessionError::ResponseTooLarge { limit: 3 })
        ));
        assert!(session.child.try_wait().unwrap().is_some());
    }
}
