//! A command that runs before OpenMango connects and stops when it disconnects: whatever opens
//! the local port the URI points at, such as `kubectl port-forward`. It runs in the login shell,
//! so PATH and the tool's own config are the terminal's.

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::oneshot;

use crate::error::{Error, Result};

const POLL: Duration = Duration::from_millis(100);
/// How long a stop waits for the command to end before killing it.
const STOP_GRACE: Duration = Duration::from_secs(2);
const KEPT_LINES: usize = 6;

/// The running command. Dropping it stops the command.
pub struct BeforeConnect {
    pid: u32,
    /// The program's name, for messages: `kubectl`.
    pub program: String,
    /// The command as given, so another connection running the same one can share it.
    pub command: String,
    status: Arc<Mutex<Option<ExitStatus>>>,
    output: Arc<Mutex<VecDeque<String>>>,
    stopped: Arc<AtomicBool>,
    /// Sent once if the command ends on its own, with why the connection closed.
    exit: Mutex<Option<oneshot::Receiver<String>>>,
    /// Our end of the pipe the wrapper waits on; the system closes it however OpenMango ends.
    #[cfg(unix)]
    #[allow(dead_code, reason = "held, never read: it only has to stay open")]
    lifeline: Option<std::process::ChildStdin>,
    /// The job the command runs in; Windows ends it when OpenMango ends.
    #[cfg(windows)]
    #[allow(dead_code, reason = "held, never read: it only has to stay open")]
    job: Option<job::Job>,
}

/// Starts `command` and waits until `endpoint` accepts a TCP connection, or, without one (an
/// SRV URI), until the command has stayed up for a second.
pub fn start(
    command: &str,
    endpoint: Option<(String, u16)>,
    timeout: Duration,
) -> Result<BeforeConnect> {
    let program = program_name(command);
    let mut child = shell(command)
        .stdin(if cfg!(unix) { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failure(format!("Couldn't start your shell to run {program}: {error}")))?;
    let output = Arc::new(Mutex::new(VecDeque::new()));
    for pipe in [
        child.stdout.take().map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let output = output.clone();
        thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(std::result::Result::ok) {
                let mut output = output.lock().unwrap();
                if output.len() == KEPT_LINES {
                    output.pop_front();
                }
                output.push_back(line);
            }
        });
    }
    let (exit_tx, exit_rx) = oneshot::channel();
    let mut handle = BeforeConnect {
        pid: child.id(),
        program: program.clone(),
        command: command.to_string(),
        status: Arc::new(Mutex::new(None)),
        output,
        stopped: Arc::new(AtomicBool::new(false)),
        exit: Mutex::new(Some(exit_rx)),
        #[cfg(unix)]
        lifeline: child.stdin.take(),
        #[cfg(windows)]
        job: job::Job::kill_on_close(&child),
    };
    thread::spawn({
        let (status, output, stopped, program) =
            (handle.status.clone(), handle.output.clone(), handle.stopped.clone(), program.clone());
        move || {
            let ended = child.wait().ok();
            *status.lock().unwrap() = ended;
            if !stopped.load(Ordering::SeqCst) {
                let lines: Vec<String> = output.lock().unwrap().iter().cloned().collect();
                let _ = exit_tx.send(explain(&program, None, Outcome::Died(code(ended)), &lines));
            }
        }
    });

    let started = Instant::now();
    let port = endpoint.as_ref().map(|(_, port)| *port);
    loop {
        if let Some(status) = *handle.status.lock().unwrap() {
            let lines = handle.lines();
            return Err(failure(explain(&program, port, Outcome::Exited(status.code()), &lines)));
        }
        let ready = match &endpoint {
            Some(endpoint) => accepts(endpoint),
            None => started.elapsed() >= Duration::from_secs(1),
        };
        if ready {
            return Ok(handle);
        }
        if started.elapsed() >= timeout {
            handle.stop();
            let lines = handle.lines();
            let message = explain(&program, port, Outcome::NotReady(timeout), &lines);
            return Err(Error::Connect {
                message: message.clone(),
                source: Box::new(Error::Timeout(message)),
            });
        }
        thread::sleep(POLL);
    }
}

impl BeforeConnect {
    /// Ends the command, and everything it started, if it's still running.
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if self.exited() {
            return;
        }
        terminate(self.pid, false);
        let started = Instant::now();
        while !self.exited() && started.elapsed() < STOP_GRACE {
            thread::sleep(POLL);
        }
        if !self.exited() {
            terminate(self.pid, true);
        }
    }

    pub fn exited(&self) -> bool {
        self.status.lock().unwrap().is_some()
    }

    /// Fires once if the command ends on its own, with why the connection closed.
    pub fn take_exit(&self) -> Option<oneshot::Receiver<String>> {
        self.exit.lock().unwrap().take()
    }

    fn lines(&self) -> Vec<String> {
        self.output.lock().unwrap().iter().cloned().collect()
    }
}

impl Drop for BeforeConnect {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accepts(endpoint: &(String, u16)) -> bool {
    let Ok(mut addresses) = (endpoint.0.as_str(), endpoint.1).to_socket_addrs() else {
        return false;
    };
    addresses.any(|address| TcpStream::connect_timeout(&address, POLL).is_ok())
}

fn code(status: Option<ExitStatus>) -> Option<i32> {
    status.and_then(|status| status.code())
}

fn failure(message: String) -> Error {
    Error::Connect { message: message.clone(), source: Box::new(Error::Parse(message)) }
}

/// `kubectl` from `kubectl port-forward …`, or `/usr/local/bin/kubectl`.
pub fn program_name(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("command");
    first.rsplit(['/', '\\']).next().unwrap_or(first).to_string()
}

/// Runs the command (`$2`) in the login shell (`$1`), and stops it when the pipe on stdin closes:
/// OpenMango never writes to it, and the system closes it when OpenMango ends, crash and force
/// quit included. Exits with the command's status otherwise.
#[cfg(unix)]
const LIFELINE: &str = r#"exec 3<&0
"$1" -l -c "$2" </dev/null &
command=$!
{ read -r _ <&3; kill -TERM 0; } &
watch=$!
wait "$command"
status=$?
kill "$watch" 2>/dev/null
exit "$status""#;

#[cfg(unix)]
fn shell(command: &str) -> std::process::Command {
    use std::os::unix::process::CommandExt as _;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut process = std::process::Command::new("/bin/sh");
    // A login shell sets the PATH the terminal has; its own group so a stop reaches what it ran.
    process.arg("-c").arg(LIFELINE).arg("sh").arg(shell).arg(command).process_group(0);
    process
}

#[cfg(windows)]
fn shell(command: &str) -> std::process::Command {
    let mut process = crate::connection::tools::tool_command("cmd");
    process.arg("/C").arg(command);
    process
}

#[cfg(unix)]
fn terminate(pid: u32, force: bool) {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: a signal to our own child's process group; no memory is involved.
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// A job that ends every process in it when its last handle closes, which Windows does
    /// however OpenMango ends. The handle, as an address so the type is `Send`.
    pub(super) struct Job(usize);

    impl Job {
        /// `None` when Windows refuses; the command then outlives a crash, as before.
        // ponytail: cmd may start the program before it joins the job; that takes cmd
        // milliseconds and joining microseconds. Spawn suspended if it ever matters.
        pub(super) fn kill_on_close(child: &std::process::Child) -> Option<Self> {
            // SAFETY: Win32 calls on a job created here and a child process we own.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Self(handle as usize);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let limited = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0;
                let joined = limited
                    && AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) != 0;
                joined.then_some(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle CreateJobObjectW returned, closed once.
            unsafe {
                CloseHandle(self.0 as HANDLE);
            }
        }
    }
}

#[cfg(windows)]
fn terminate(pid: u32, _force: bool) {
    let _ = crate::connection::tools::tool_command("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output();
}

#[cfg(not(any(unix, windows)))]
fn terminate(_pid: u32, _force: bool) {}

#[cfg(not(any(unix, windows)))]
fn shell(command: &str) -> std::process::Command {
    let mut process = std::process::Command::new("sh");
    process.arg("-c").arg(command);
    process
}

pub enum Outcome {
    /// Ended before the port accepted connections.
    Exited(Option<i32>),
    /// Still running, but the port never accepted a connection.
    NotReady(Duration),
    /// Ended after OpenMango had connected.
    Died(Option<i32>),
}

fn shell_name() -> String {
    if cfg!(windows) {
        return "cmd".into();
    }
    std::env::var("SHELL").ok().map(|shell| program_name(&shell)).unwrap_or_else(|| "sh".into())
}

/// A plain first line for what went wrong, then what the command said.
pub fn explain(program: &str, port: Option<u16>, outcome: Outcome, output: &[String]) -> String {
    let said = output.join("\n").to_ascii_lowercase();
    let code = |code: Option<i32>| match code {
        Some(code) => format!(" (code {code})"),
        None => String::new(),
    };
    let port_text = port.map(|port| format!("port {port}")).unwrap_or_else(|| "the port".into());
    let first = match outcome {
        Outcome::Exited(_) if said.contains("address already in use") => format!(
            "{port_text} is already in use, so {program} couldn't listen on it. Stop what's \
             using it, or forward another port here and in the URI."
        ),
        Outcome::Exited(exit)
            if exit == Some(127)
                || said.contains("command not found")
                || said.contains("unknown command")
                || said.contains("not recognized as an internal") =>
        {
            format!(
                "Your shell ({}) couldn't find {program}. Install it, or add it to the PATH \
                 your shell sets when you log in.",
                shell_name()
            )
        }
        Outcome::Exited(exit) => {
            format!("{program} exited{} before {port_text} accepted connections.", code(exit))
        }
        Outcome::NotReady(timeout) => format!(
            "{program} is running, but {port_text} didn't accept connections within {} seconds. \
             Check that the command forwards to that port and that the URI uses it.",
            timeout.as_secs()
        ),
        Outcome::Died(exit) => {
            format!("{program} stopped{}, so the connection was closed.", code(exit))
        }
    };
    let first = capitalize(&first);
    if output.is_empty() { first } else { format!("{first}\n\nIt said:\n{}", output.join("\n")) }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_common_failures_plainly() {
        let in_use = explain(
            "kubectl",
            Some(27018),
            Outcome::Exited(Some(1)),
            &["Unable to listen on port 27018: bind: address already in use".into()],
        );
        assert!(in_use.starts_with("Port 27018 is already in use, so kubectl couldn't listen"));
        assert!(
            in_use.ends_with(
                "It said:\nUnable to listen on port 27018: bind: address already in use"
            )
        );

        let missing = explain("kubectl", Some(27018), Outcome::Exited(Some(127)), &[]);
        assert!(missing.contains("couldn't find kubectl"), "{missing}");
        let fish = explain(
            "kubectl",
            None,
            Outcome::Exited(Some(1)),
            &["fish: Unknown command: kubectl".into()],
        );
        assert!(fish.contains("couldn't find kubectl"), "{fish}");

        assert!(
            explain("kubectl", Some(27018), Outcome::Exited(Some(1)), &["pod not running".into()])
                .starts_with("Kubectl exited (code 1) before port 27018 accepted connections.")
        );
        assert!(
            explain("kubectl", None, Outcome::NotReady(Duration::from_secs(15)), &[])
                .contains("the port didn't accept connections within 15 seconds")
        );
        assert_eq!(
            explain(
                "kubectl",
                None,
                Outcome::Died(Some(1)),
                &["error: lost connection to pod".into()]
            ),
            "Kubectl stopped (code 1), so the connection was closed.\n\nIt said:\nerror: lost connection to pod"
        );
        assert_eq!(program_name("/usr/local/bin/kubectl port-forward x"), "kubectl");
        assert_eq!(program_name("  "), "command");
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_the_port_reports_early_exits_and_stops_what_it_started() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = Some(("127.0.0.1".to_string(), port));

        // The port accepts, so it's ready at once; a stop ends the command.
        let mut running = start("sleep 30", endpoint.clone(), Duration::from_secs(5)).unwrap();
        assert_eq!(running.program, "sleep");
        assert!(!running.exited());
        running.stop();
        assert!(running.exited(), "stopped");

        // Ended before the port was ready, with what it said.
        let closed = Some(("127.0.0.1".to_string(), free_port()));
        let error = start("echo nope >&2; exit 3", closed.clone(), Duration::from_secs(5))
            .err()
            .expect("an early exit fails");
        assert!(error.to_string().contains("exited (code 3)"), "{error}");
        assert!(error.to_string().ends_with("It said:\nnope"), "{error}");

        // Never ready: stopped, and the failure says so.
        let error = start("sleep 30", closed, Duration::from_millis(400)).err().unwrap();
        assert!(error.to_string().contains("didn't accept connections"), "{error}");
        assert!(error.is_transient(), "worth trying again");

        // Ending on its own after a connect is reported once.
        let short = start("sleep 0.3", endpoint, Duration::from_secs(5)).unwrap();
        let exit = short.take_exit().unwrap();
        let reason = futures::executor::block_on(exit).unwrap();
        assert!(reason.starts_with("Sleep stopped (code 0)"), "{reason}");
        assert!(short.take_exit().is_none());
    }

    /// Closing OpenMango's end of the pipe is what the system does when OpenMango crashes or is
    /// force-quit: the command stops without a stop.
    #[cfg(unix)]
    #[test]
    fn the_command_stops_when_openmango_ends_without_stopping_it() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Some(("127.0.0.1".to_string(), listener.local_addr().unwrap().port()));
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let command = format!("sh -c 'echo $$ > {}; exec sleep 30'", pid_file.display());
        let mut running = start(&command, endpoint, Duration::from_secs(5)).unwrap();

        let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
        let until = |done: &dyn Fn() -> bool| {
            let started = Instant::now();
            while !done() && started.elapsed() < Duration::from_secs(5) {
                thread::sleep(POLL);
            }
            done()
        };
        let read_pid = || std::fs::read_to_string(&pid_file).ok()?.trim().parse::<i32>().ok();
        assert!(until(&|| read_pid().is_some()), "the command wrote its pid");
        let pid = read_pid().unwrap();
        assert!(alive(pid));

        drop(running.lifeline.take());
        assert!(until(&|| !alive(pid)), "the command ended with OpenMango");
        assert!(until(&|| running.exited()));
    }

    #[cfg(windows)]
    #[test]
    fn the_command_stops_when_openmango_ends_without_stopping_it() {
        let mut running =
            start("ping -n 30 127.0.0.1 > nul", None, Duration::from_secs(5)).unwrap();
        assert!(running.job.is_some(), "the command runs in a job");
        assert!(!running.exited());
        // Windows closes the job's handle when OpenMango ends, however it ends.
        drop(running.job.take());
        let started = Instant::now();
        while !running.exited() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(POLL);
        }
        assert!(running.exited(), "the command ended with its job");
    }

    #[cfg(unix)]
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }
}
