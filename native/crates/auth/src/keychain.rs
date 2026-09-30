use std::fmt;
use std::io::{self, Write};
use std::process::{Command, Stdio};

pub const KEYCHAIN_SERVICE: &str = "slopify-spotify-refresh-token";

// The CLI, not a Keychain API linked into the app: the app is ad-hoc signed, so only items
// trusting /usr/bin/security survive a rebuild without a prompt (docs/spec/v1.md, Auth).
const SECURITY_BIN: &str = "/usr/bin/security";
const ITEM_NOT_FOUND: i32 = 44;

/// Where the refresh token lives between launches.
pub trait Keychain: Send + Sync {
    fn read(&self) -> Result<Option<String>, KeychainError>;
    fn write(&self, user_id: &str, token: &str) -> Result<(), KeychainError>;
    fn delete(&self) -> Result<(), KeychainError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeychainError(pub String);

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeychainError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `security` with these args, feeding `stdin` if given.
pub type SecurityRunner = dyn Fn(&[&str], Option<&str>) -> io::Result<SecurityOutput> + Send + Sync;

pub fn run_security(args: &[&str], stdin: Option<&str>) -> io::Result<SecurityOutput> {
    let mut child = Command::new(SECURITY_BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Dropping the handle closes stdin, so security sees EOF either way.
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin.unwrap_or("").as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(SecurityOutput {
        // A signal kill has no code; report it as a failure rather than success.
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// The login Keychain through `/usr/bin/security`.
pub struct SecurityKeychain {
    run: Box<SecurityRunner>,
}

impl SecurityKeychain {
    pub fn new() -> Self {
        Self::with_runner(run_security)
    }

    pub fn with_runner(
        run: impl Fn(&[&str], Option<&str>) -> io::Result<SecurityOutput> + Send + Sync + 'static,
    ) -> Self {
        Self { run: Box::new(run) }
    }

    fn run(
        &self,
        op: &str,
        args: &[&str],
        stdin: Option<&str>,
    ) -> Result<SecurityOutput, KeychainError> {
        (self.run)(args, stdin)
            .map_err(|e| KeychainError(format!("security {op} failed to run: {e}")))
    }
}

impl Default for SecurityKeychain {
    fn default() -> Self {
        Self::new()
    }
}

fn fail(op: &str, r: &SecurityOutput) -> KeychainError {
    KeychainError(format!(
        "security {op} exited {}: {}",
        r.code,
        r.stderr.trim()
    ))
}

impl Keychain for SecurityKeychain {
    fn read(&self) -> Result<Option<String>, KeychainError> {
        let op = "find-generic-password";
        let r = self.run(op, &[op, "-s", KEYCHAIN_SERVICE, "-w"], None)?;
        if r.code == ITEM_NOT_FOUND {
            return Ok(None);
        }
        if r.code != 0 {
            return Err(fail(op, &r));
        }
        let value = r.stdout.strip_suffix('\n').unwrap_or(&r.stdout);
        let value = value.strip_suffix('\r').unwrap_or(value);
        Ok((!value.is_empty()).then(|| value.to_string()))
    }

    // `-w` with no value reads from the tty, never from a pipe, so a piped value stores an empty
    // secret. `security -i` takes whole commands on stdin instead; the token still never hits argv.
    fn write(&self, user_id: &str, token: &str) -> Result<(), KeychainError> {
        let command = [
            "add-generic-password",
            "-a",
            &quote(user_id),
            "-s",
            KEYCHAIN_SERVICE,
            "-U",
            "-T",
            SECURITY_BIN,
            "-w",
            &quote(token),
        ]
        .join(" ");
        let r = self.run(
            "add-generic-password",
            &["-i"],
            Some(&format!("{command}\n")),
        )?;
        if r.code != 0 {
            return Err(fail("add-generic-password", &r));
        }
        Ok(())
    }

    fn delete(&self) -> Result<(), KeychainError> {
        let op = "delete-generic-password";
        let r = self.run(op, &[op, "-s", KEYCHAIN_SERVICE], None)?;
        if r.code != 0 && r.code != ITEM_NOT_FOUND {
            return Err(fail(op, &r));
        }
        Ok(())
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<(Vec<String>, Option<String>)>>>;

    fn fake(result: SecurityOutput) -> (SecurityKeychain, Calls) {
        let calls: Calls = Arc::default();
        let seen = calls.clone();
        let kc = SecurityKeychain::with_runner(move |args, stdin| {
            seen.lock().unwrap().push((
                args.iter().map(|a| a.to_string()).collect(),
                stdin.map(str::to_string),
            ));
            Ok(result.clone())
        });
        (kc, calls)
    }

    fn out(code: i32, stdout: &str, stderr: &str) -> SecurityOutput {
        SecurityOutput {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn names_the_service_after_the_spec() {
        assert_eq!(KEYCHAIN_SERVICE, "slopify-spotify-refresh-token");
    }

    #[test]
    fn read_runs_find_generic_password_w_and_returns_the_trimmed_value() {
        let (kc, calls) = fake(out(0, "tok-123\n", ""));
        assert_eq!(kc.read(), Ok(Some("tok-123".into())));
        assert_eq!(
            calls.lock().unwrap()[0].0,
            ["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"]
        );
    }

    #[test]
    fn read_treats_an_empty_stored_value_as_no_token() {
        let (kc, _) = fake(out(0, "\n", ""));
        assert_eq!(kc.read(), Ok(None));
    }

    #[test]
    fn read_returns_none_when_the_item_is_missing() {
        let (kc, _) = fake(out(
            44,
            "",
            "The specified item could not be found in the keychain.",
        ));
        assert_eq!(kc.read(), Ok(None));
    }

    #[test]
    fn read_rethrows_other_failures() {
        let (kc, _) = fake(out(36, "", "User interaction is not allowed."));
        assert!(kc.read().unwrap_err().0.contains("User interaction"));
    }

    #[test]
    fn write_feeds_add_generic_password_to_security_i_on_stdin_with_the_token_quoted() {
        let (kc, calls) = fake(out(0, "", ""));
        kc.write("user-1", "tok-123").unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0, ["-i"]);
        assert_eq!(
            calls[0].1.as_deref(),
            Some(
                "add-generic-password -a 'user-1' -s slopify-spotify-refresh-token -U -T /usr/bin/security -w 'tok-123'\n"
            )
        );
    }

    #[test]
    fn write_never_puts_the_token_in_argv() {
        let (kc, calls) = fake(out(0, "", ""));
        kc.write("user-1", "tok-123").unwrap();
        assert!(!calls.lock().unwrap()[0].0.join(" ").contains("tok-123"));
    }

    #[test]
    fn write_escapes_single_quotes_in_the_token() {
        let (kc, calls) = fake(out(0, "", ""));
        kc.write("user-1", "to'k").unwrap();
        let stdin = calls.lock().unwrap()[0].1.clone().unwrap();
        assert!(stdin.contains(r"-w 'to'\''k'"));
    }

    #[test]
    fn write_fails_on_a_nonzero_exit() {
        let (kc, _) = fake(out(1, "", "nope"));
        assert!(kc.write("user-1", "tok").unwrap_err().0.contains("nope"));
    }

    #[test]
    fn delete_runs_delete_generic_password() {
        let (kc, calls) = fake(out(0, "", ""));
        kc.delete().unwrap();
        assert_eq!(
            calls.lock().unwrap()[0].0,
            ["delete-generic-password", "-s", KEYCHAIN_SERVICE]
        );
    }

    #[test]
    fn delete_treats_a_missing_item_as_already_deleted() {
        let (kc, _) = fake(out(44, "", ""));
        assert_eq!(kc.delete(), Ok(()));
    }

    // Read-only against a service name nothing ever writes, so the real item is never touched.
    #[cfg(target_os = "macos")]
    #[test]
    fn real_security_reports_a_missing_item_as_exit_44() {
        let r = run_security(
            &[
                "find-generic-password",
                "-s",
                "slopify-auth-test-never-written",
                "-w",
            ],
            None,
        )
        .unwrap();
        assert_eq!(r.code, ITEM_NOT_FOUND);
    }
}
