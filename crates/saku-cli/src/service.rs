//! Manage the user-level systemd **Host Service** unit (`saku.service`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SERVICE_UNIT_NAME: &str = "saku.service";

/// Resolved paths and content for the Host Service unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePaths {
    pub unit_path: PathBuf,
    pub binary_path: PathBuf,
}

/// Observed Host Service state (best-effort from systemd/loginctl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub installed: bool,
    pub enabled: bool,
    pub active: bool,
    pub linger: bool,
}

/// Resolve unit and binary paths under `home`.
pub fn service_paths(home: &Path) -> ServicePaths {
    ServicePaths {
        unit_path: home.join(".config/systemd/user").join(SERVICE_UNIT_NAME),
        binary_path: home.join(".local/bin/saku"),
    }
}

/// Render the unit file body for `home`.
pub fn unit_file_content(home: &Path) -> String {
    let paths = service_paths(home);
    format!(
        "[Unit]\n\
         Description=Saku Discord coding agent\n\
         \n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        paths.binary_path.display()
    )
}

/// Whether the unit file exists at the default location.
pub fn is_unit_installed(home: &Path) -> bool {
    service_paths(home).unit_path.is_file()
}

fn require_linux() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        Err("Host Service management requires Linux".into())
    }
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())
}

fn run_systemctl(args: &[&str]) -> Result<(), String> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run systemctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {} failed", args.join(" ")))
    }
}

fn systemctl_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run systemctl: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("systemctl {} failed", args.join(" ")))
    }
}

fn run_loginctl(args: &[&str]) -> Result<(), String> {
    let status = Command::new("loginctl")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run loginctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("loginctl {} failed", args.join(" ")))
    }
}

fn loginctl_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("loginctl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run loginctl: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("loginctl {} failed", args.join(" ")))
    }
}

fn current_user() -> Result<String, String> {
    std::env::var("USER").map_err(|_| "could not determine USER".to_string())
}

fn query_unit_enabled() -> bool {
    systemctl_output(&["--user", "is-enabled", SERVICE_UNIT_NAME])
        .is_ok_and(|state| state == "enabled")
}

fn query_unit_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SERVICE_UNIT_NAME])
        .status()
        .is_ok_and(|status| status.success())
}

fn query_linger() -> bool {
    let user = match current_user() {
        Ok(user) => user,
        Err(_) => return false,
    };
    loginctl_output(&["show-user", &user, "-p", "Linger", "--value"])
        .is_ok_and(|value| value == "yes")
}

/// Collect Host Service status for `home`.
pub fn collect_status(home: &Path) -> ServiceStatus {
    ServiceStatus {
        installed: is_unit_installed(home),
        enabled: query_unit_enabled(),
        active: query_unit_active(),
        linger: query_linger(),
    }
}

/// Write the unit file and enable linger (does not enable/start the unit).
pub fn install() -> Result<(), String> {
    require_linux()?;
    let home = home_dir()?;
    let paths = service_paths(&home);
    fs::create_dir_all(
        paths
            .unit_path
            .parent()
            .ok_or_else(|| "invalid unit path".to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(&paths.unit_path, unit_file_content(&home)).map_err(|e| e.to_string())?;
    let user = current_user()?;
    run_loginctl(&["enable-linger", &user])?;
    run_systemctl(&["--user", "daemon-reload"])?;
    println!("Installed {}.", SERVICE_UNIT_NAME);
    println!("Run `saku service enable` when you are ready to start it.");
    Ok(())
}

/// Stop, disable, remove the unit, and reload systemd (linger stays enabled).
pub fn uninstall() -> Result<(), String> {
    require_linux()?;
    let home = home_dir()?;
    let paths = service_paths(&home);
    if query_unit_active() {
        let _ = run_systemctl(&["--user", "stop", SERVICE_UNIT_NAME]);
    }
    let _ = run_systemctl(&["--user", "disable", SERVICE_UNIT_NAME]);
    if paths.unit_path.exists() {
        fs::remove_file(&paths.unit_path).map_err(|e| e.to_string())?;
    }
    run_systemctl(&["--user", "daemon-reload"])?;
    println!("Uninstalled {}.", SERVICE_UNIT_NAME);
    Ok(())
}

/// Enable the unit and start it now.
pub fn enable() -> Result<(), String> {
    require_linux()?;
    let home = home_dir()?;
    if !is_unit_installed(&home) {
        return Err(format!(
            "{} is not installed; run `saku service install` first",
            SERVICE_UNIT_NAME
        ));
    }
    run_systemctl(&["--user", "enable", "--now", SERVICE_UNIT_NAME])?;
    println!("Enabled and started {}.", SERVICE_UNIT_NAME);
    Ok(())
}

/// Disable the unit and stop it if active.
pub fn disable() -> Result<(), String> {
    require_linux()?;
    run_systemctl(&["--user", "disable", SERVICE_UNIT_NAME])?;
    if query_unit_active() {
        run_systemctl(&["--user", "stop", SERVICE_UNIT_NAME])?;
    }
    println!("Disabled {}.", SERVICE_UNIT_NAME);
    Ok(())
}

/// Print Host Service status to stdout.
pub fn status() -> Result<(), String> {
    require_linux()?;
    let home = home_dir()?;
    let state = collect_status(&home);
    println!("installed: {}", yes_no(state.installed));
    if !state.installed {
        println!("hint: run `saku service install` to create the unit file");
        return Ok(());
    }
    println!("enabled: {}", yes_no(state.enabled));
    println!("active: {}", yes_no(state.active));
    println!("linger: {}", yes_no(state.linger));
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Whether `saku.service` is currently active.
fn is_service_active() -> bool {
    query_unit_active()
}

/// Restart the Host Service when it is already active (used by `saku update`).
pub fn restart_service_if_active() {
    if is_service_active() {
        match run_systemctl(&["--user", "restart", SERVICE_UNIT_NAME]) {
            Ok(()) => println!("Restarted {}.", SERVICE_UNIT_NAME),
            Err(_) => eprintln!("warning: failed to restart {}", SERVICE_UNIT_NAME),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_file_content_matches_install_template() {
        let home = PathBuf::from("/home/alice");
        let content = unit_file_content(&home);
        assert!(content.contains("Description=Saku Discord coding agent"));
        assert!(content.contains("ExecStart=/home/alice/.local/bin/saku"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("WantedBy=default.target"));
    }

    #[test]
    fn service_paths_resolve_under_home() {
        let home = PathBuf::from("/home/alice");
        let paths = service_paths(&home);
        assert_eq!(
            paths.unit_path,
            PathBuf::from("/home/alice/.config/systemd/user/saku.service")
        );
        assert_eq!(
            paths.binary_path,
            PathBuf::from("/home/alice/.local/bin/saku")
        );
    }

    #[test]
    fn is_unit_installed_reflects_file_presence() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(!is_unit_installed(home));
        let unit_path = service_paths(home).unit_path;
        fs::create_dir_all(unit_path.parent().unwrap()).unwrap();
        fs::write(&unit_path, unit_file_content(home)).unwrap();
        assert!(is_unit_installed(home));
    }

    #[test]
    fn collect_status_installed_without_systemd_queries() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let unit_path = service_paths(home).unit_path;
        fs::create_dir_all(unit_path.parent().unwrap()).unwrap();
        fs::write(&unit_path, "stub").unwrap();
        let state = collect_status(home);
        assert!(state.installed);
    }

    #[test]
    fn unit_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let unit_path = service_paths(home).unit_path;
        fs::create_dir_all(unit_path.parent().unwrap()).unwrap();
        fs::write(&unit_path, unit_file_content(home)).unwrap();
        let written = fs::read_to_string(unit_path).unwrap();
        assert_eq!(written, unit_file_content(home));
    }
}
