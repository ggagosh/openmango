//! The system entry that starts `openmango --run-due-tasks` about every 15 minutes, so tasks can
//! run while OpenMango is closed.
//!
//! On macOS it is a launch agent inside the app bundle, at
//! `Contents/Library/LaunchAgents/com.openmango.app.tasks.plist`, registered with `SMAppService`
//! so it shows in System Settings → General → Login Items. Windows and Linux come later.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunnerStatus {
    /// Registered and allowed to run.
    Enabled,
    /// Registered, but switched off in Login Items or not allowed yet.
    NeedsApproval,
    #[default]
    NotRegistered,
    /// This OpenMango can't have one, and why.
    Unavailable(&'static str),
}

#[cfg(target_os = "macos")]
pub use mac::{open_login_items, register, status, unregister};

#[cfg(not(target_os = "macos"))]
pub fn status() -> RunnerStatus {
    RunnerStatus::Unavailable("Running while OpenMango is closed comes to Windows and Linux later.")
}

#[cfg(not(target_os = "macos"))]
pub fn register() -> Result<RunnerStatus, String> {
    Ok(status())
}

#[cfg(not(target_os = "macos"))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn open_login_items() {}

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
    pub fn open_login_items() {
        if let Some(class) = AnyClass::get(c"SMAppService")
            && class.responds_to(sel!(openSystemSettingsLoginItems))
        {
            let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
        }
    }
}
