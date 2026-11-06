//! Opt-in execution of the person's own launcher. No document text, instruction or token is
//! passed in arguments, a shell command, or logs. The launcher connects through the normal key file.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use super::hub::AgentHub;

#[derive(Default)]
pub struct Launcher {
    child: Option<Child>,
}

/// An explicit absolute executable, or an opt-in launcher beside the app executable.
fn configured(variable: &str) -> Result<Option<PathBuf>, String> {
    if let Some(path) = std::env::var_os(variable).filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() || !path.is_file() {
            return Err(format!("{variable} must name an absolute launcher file"));
        }
        return Ok(Some(path));
    }
    let name = if cfg!(windows) { "launch.cmd" } else { "launch" };
    let path = std::env::current_exe().ok().and_then(|path| path.parent().map(|parent| parent.join("agent").join(name)));
    Ok(path.filter(|path| path.is_file()))
}

/// Missing configuration is the default and never starts a process.
pub fn available(variable: &str) -> bool {
    configured(variable).ok().flatten().is_some()
}

impl Launcher {
    /// Start only after explicit Send, only when nobody listens, and never twice concurrently.
    pub fn start(&mut self, variable: &str, hub: &AgentHub) -> Result<(), String> {
        if !hub.agents().is_empty() {
            return Ok(());
        }
        if let Some(child) = &mut self.child {
            match child.try_wait().map_err(|_| "cannot check the configured agent launcher")? {
                None => return Ok(()),
                Some(_) => self.child = None,
            }
        }
        let Some(program) = configured(variable)? else { return Ok(()) };
        self.spawn(&program)
    }

    /// Invoke an executable directly with no document-derived arguments or shell expansion.
    fn spawn(&mut self, program: &std::path::Path) -> Result<(), String> {
        let mut command = Command::new(program);
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(parent) = program.parent() {
            command.current_dir(parent);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        self.child = Some(command.spawn().map_err(|_| "could not start the configured agent launcher")?);
        Ok(())
    }
}
