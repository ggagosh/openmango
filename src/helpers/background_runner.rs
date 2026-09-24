//! The system entry that starts `openmango --run-due-tasks` about every 15 minutes, so tasks can
//! run while OpenMango is closed.
//!
//! On macOS it is a launch agent inside the app bundle, at
//! `Contents/Library/LaunchAgents/com.openmango.app.tasks.plist`, registered with `SMAppService`
//! so it shows in System Settings → General → Login Items. On Windows it is a Task Scheduler task,
//! `OpenMango\Run due tasks`. Linux comes later.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunnerStatus {
    /// Registered and allowed to run.
    Enabled,
    /// Registered, but switched off by the user or not allowed yet.
    NeedsApproval,
    #[default]
    NotRegistered,
    /// This OpenMango can't have one, and why.
    Unavailable(&'static str),
}

/// The button that opens where the entry is switched on or off.
pub const OPEN_SETTINGS: &str =
    if cfg!(windows) { "Open Task Scheduler" } else { "Open Login Items" };
/// How to switch the entry back on.
pub const SWITCH_ON: &str = if cfg!(windows) {
    "Enable “Run due tasks” in Task Scheduler, in the OpenMango folder."
} else {
    "Switch OpenMango on in System Settings, under Login Items."
};
/// Where the entry is listed.
pub const LISTED_IN: &str = if cfg!(windows) {
    "It's listed in Task Scheduler, in the OpenMango folder."
} else {
    "It's listed in System Settings under Login Items."
};

#[cfg(target_os = "macos")]
pub use mac::{open_settings, register, status, unregister};
#[cfg(windows)]
pub use win::{open_settings, register, status, unregister};

#[cfg(not(any(target_os = "macos", windows)))]
pub fn status() -> RunnerStatus {
    RunnerStatus::Unavailable("Running while OpenMango is closed comes to Linux later.")
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn register() -> Result<RunnerStatus, String> {
    Ok(status())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn open_settings() {}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, sel};
    use objc2_foundation::{NSBundle, NSError, NSString};

    use super::RunnerStatus;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    const PLIST: &str = "com.openmango.app.tasks.plist";

    /// The agent's service, or why there can't be one.
    fn service() -> Result<Retained<AnyObject>, &'static str> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return Err("Needs OpenMango installed as an app; development builds can't.");
        }
        // SMAppService arrived in macOS 13.
        let class = AnyClass::get(c"SMAppService").ok_or("Needs macOS 13 or later.")?;
        let name = NSString::from_str(PLIST);
        let service: Option<Retained<AnyObject>> =
            unsafe { msg_send![class, agentServiceWithPlistName: &*name] };
        service.ok_or("The app is missing its launch agent.")
    }

    pub fn status() -> RunnerStatus {
        let service = match service() {
            Ok(service) => service,
            Err(reason) => return RunnerStatus::Unavailable(reason),
        };
        // SMAppServiceStatus: not registered, enabled, requires approval, not found.
        let status: isize = unsafe { msg_send![&service, status] };
        match status {
            1 => RunnerStatus::Enabled,
            2 => RunnerStatus::NeedsApproval,
            3 => RunnerStatus::Unavailable("The app is missing its launch agent."),
            _ => RunnerStatus::NotRegistered,
        }
    }

    pub fn register() -> Result<RunnerStatus, String> {
        let service = service().map_err(str::to_string)?;
        let registered: Result<(), Retained<NSError>> =
            unsafe { msg_send![&service, registerAndReturnError: _] };
        match registered {
            Ok(()) => Ok(status()),
            // Already registered but switched off: macOS says so with an error, and the status
            // shows it.
            Err(_) if status() == RunnerStatus::NeedsApproval => Ok(RunnerStatus::NeedsApproval),
            Err(error) => Err(error.localizedDescription().to_string()),
        }
    }

    pub fn unregister() -> Result<(), String> {
        let service = service().map_err(str::to_string)?;
        let unregistered: Result<(), Retained<NSError>> =
            unsafe { msg_send![&service, unregisterAndReturnError: _] };
        unregistered.map_err(|error| error.localizedDescription().to_string())
    }

    /// Opens System Settings at Login Items, where the agent is switched on or off.
    pub fn open_settings() {
        if let Some(class) = AnyClass::get(c"SMAppService")
            && class.responds_to(sel!(openSystemSettingsLoginItems))
        {
            let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
        }
    }
}

#[cfg(windows)]
mod win {
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use super::RunnerStatus;

    const TASK: &str = r"OpenMango\Run due tasks";
    const DEV: &str = "Development builds don't add the Task Scheduler task.";

    /// A program from System32, so nothing earlier on the search path stands in for it.
    fn system32(program: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join("System32").join(program)
    }

    /// Runs `schtasks` without flashing a console window.
    fn schtasks(args: &[&str]) -> std::io::Result<Output> {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new(system32("schtasks.exe")).args(args).creation_flags(CREATE_NO_WINDOW).output()
    }

    fn succeeded(output: std::io::Result<Output>) -> Result<(), String> {
        let output = output.map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }

    // Development builds are console programs, so each start would flash a window, and a running
    // runner would keep cargo from replacing the program. They're tried with `--run-due-tasks`.
    pub fn status() -> RunnerStatus {
        if cfg!(debug_assertions) { RunnerStatus::Unavailable(DEV) } else { task_status(TASK) }
    }

    pub fn register() -> Result<RunnerStatus, String> {
        if cfg!(debug_assertions) {
            return Ok(RunnerStatus::Unavailable(DEV));
        }
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        create_task(TASK, &exe)?;
        Ok(task_status(TASK))
    }

    pub fn unregister() -> Result<(), String> {
        if cfg!(debug_assertions) {
            return Ok(());
        }
        delete_task(TASK)
    }

    /// Opens Task Scheduler, where the task is enabled or disabled.
    pub fn open_settings() {
        // `start` goes through the shell, which opens .msc files and asks to elevate if needed.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new(system32("cmd.exe"))
            .args(["/c", "start", "", "taskschd.msc"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    pub(super) fn task_status(name: &str) -> RunnerStatus {
        match schtasks(&["/Query", "/TN", name, "/XML"]) {
            Ok(output) if output.status.success() => {
                // The XML may come as UTF-16; its markup is ASCII once the zero bytes are gone.
                let xml: Vec<u8> = output.stdout.into_iter().filter(|byte| *byte != 0).collect();
                if String::from_utf8_lossy(&xml).contains("<Enabled>false</Enabled>") {
                    RunnerStatus::NeedsApproval
                } else {
                    RunnerStatus::Enabled
                }
            }
            _ => RunnerStatus::NotRegistered,
        }
    }

    // ponytail: the task keeps the path it was added with; an install moved to another folder
    // leaves it pointing at the old one. Compare `<Command>` with `current_exe` if that happens.
    pub(super) fn create_task(name: &str, exe: &Path) -> Result<(), String> {
        let file = std::env::temp_dir().join(format!("openmango-task-{}.xml", std::process::id()));
        // schtasks reads task XML as UTF-16, with its byte order mark.
        let xml = super::task_xml(&exe.to_string_lossy());
        let bytes: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain(xml.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        std::fs::write(&file, bytes).map_err(|error| error.to_string())?;
        let created = schtasks(&["/Create", "/TN", name, "/XML", &file.to_string_lossy(), "/F"]);
        let _ = std::fs::remove_file(&file);
        succeeded(created)
    }

    pub(super) fn delete_task(name: &str) -> Result<(), String> {
        succeeded(schtasks(&["/Delete", "/TN", name, "/F"]))
    }

    #[cfg(test)]
    pub(super) fn set_enabled(name: &str, enabled: bool) -> Result<(), String> {
        succeeded(schtasks(&["/Change", "/TN", name, if enabled { "/ENABLE" } else { "/DISABLE" }]))
    }
}

/// The Task Scheduler definition. It runs as the signed-in user, only while they're signed in,
/// which is what lets it read the passwords Credential Manager keeps for them.
///
/// - Starts one minute past each quarter hour, so a task due on the quarter hour runs a minute
///   later, and as soon as possible after a start missed while the computer slept or was off.
/// - Runs on battery, and keeps running when the computer goes on battery.
/// - Starts nothing while it still runs, and stops after 25 hours: a run stops itself at 24.
#[cfg(any(windows, test))]
fn task_xml(exe: &str) -> String {
    let exe = exe.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Runs OpenMango tasks that are due while OpenMango is closed. OpenMango adds and removes this entry.</Description>
  </RegistrationInfo>
  <Triggers>
    <TimeTrigger>
      <Repetition>
        <Interval>PT15M</Interval>
      </Repetition>
      <StartBoundary>2000-01-01T00:01:00</StartBoundary>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT25H</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--run-due-tasks</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_escapes_the_program_path() {
        let xml = task_xml(r"C:\Users\Tom & <Jerry>\OpenMango.exe");
        assert!(xml.contains(r"<Command>C:\Users\Tom &amp; &lt;Jerry&gt;\OpenMango.exe</Command>"));
        assert_eq!(xml.matches("<Enabled>").count(), 1, "status reads the only <Enabled>");
    }

    /// Adds a throwaway task with the real `schtasks`, then switches it off and deletes it. Runs
    /// on the Windows CI machines.
    #[cfg(windows)]
    #[test]
    fn task_scheduler_entry_round_trip() {
        use super::win::{create_task, delete_task, set_enabled, task_status};

        let name = format!(r"OpenMango\Tests\Run due tasks {}", std::process::id());
        struct Delete<'a>(&'a str);
        impl Drop for Delete<'_> {
            fn drop(&mut self) {
                let _ = delete_task(self.0);
            }
        }

        assert_eq!(task_status(&name), RunnerStatus::NotRegistered);
        create_task(&name, &std::env::current_exe().unwrap()).unwrap();
        let _delete = Delete(&name);
        assert_eq!(task_status(&name), RunnerStatus::Enabled);
        set_enabled(&name, false).unwrap();
        assert_eq!(task_status(&name), RunnerStatus::NeedsApproval);
        set_enabled(&name, true).unwrap();
        assert_eq!(task_status(&name), RunnerStatus::Enabled);
        delete_task(&name).unwrap();
        assert_eq!(task_status(&name), RunnerStatus::NotRegistered);
    }
}
