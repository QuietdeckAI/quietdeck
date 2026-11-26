//! `quietdeck-node install` and `uninstall`: the node as a launchd agent (macOS) or a systemd user service (Linux).
//! The templates are deploy/com.quietdeck.node.plist and deploy/quietdeck-node.service.

use std::path::{Path, PathBuf};
use std::process::Command;

const PLIST: &str = include_str!("../../../deploy/com.quietdeck.node.plist");
const UNIT: &str = include_str!("../../../deploy/quietdeck-node.service");
const LABEL: &str = "com.quietdeck.node";

pub struct Values {
    pub bin: PathBuf,
    pub path: String,
    pub log: PathBuf,
}

pub fn render(template: &str, v: &Values) -> String {
    template
        .replace("{{BIN}}", &v.bin.display().to_string())
        .replace("{{PATH}}", &v.path)
        .replace("{{LOG}}", &v.log.display().to_string())
}

fn service_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
    } else {
        home.join(".config/systemd/user/quietdeck-node.service")
    }
}

fn run(cmd: &mut Command) -> anyhow::Result<()> {
    let out = cmd.output()?;
    if !out.status.success() {
        anyhow::bail!("{:?}: {}", cmd, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn uid() -> anyhow::Result<String> {
    let out = Command::new("id").arg("-u").output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn install(home: &Path, start: bool) -> anyhow::Result<()> {
    // The node needs its config before it can run, or the service would restart forever.
    let config = crate::config::config_path();
    crate::config::NodeConfig::load(&config)?;
    let bin = std::env::current_exe()?.canonicalize()?;
    let values = Values {
        bin,
        path: std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        log: if cfg!(target_os = "macos") {
            home.join("Library/Logs/quietdeck-node.log")
        } else {
            crate::config::state_dir().join("node.log")
        },
    };
    let file = service_file(home);
    std::fs::create_dir_all(file.parent().expect("a service file has a directory"))?;
    let template = if cfg!(target_os = "macos") { PLIST } else { UNIT };
    std::fs::write(&file, render(template, &values))?;
    println!("Wrote {}.", file.display());
    if !start {
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        let domain = format!("gui/{}", uid()?);
        // Replace a loaded copy, so a reinstall picks up a new binary or PATH. bootout returns before the old job is
        // fully gone, and a bootstrap in that gap fails with "5: Input/output error" (26 Nov 2026, on a Mac): wait for
        // it to go, then bootstrap, trying again a few times.
        let target = format!("{domain}/{LABEL}");
        let _ = Command::new("launchctl").args(["bootout", &target]).output();
        for _ in 0..40 {
            let loaded = Command::new("launchctl").args(["print", &target]).output().is_ok_and(|o| o.status.success());
            if !loaded {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let mut tries = 0;
        loop {
            tries += 1;
            match run(Command::new("launchctl").args(["bootstrap", &domain]).arg(&file)) {
                Ok(()) => break,
                Err(e) if tries < 5 => {
                    eprintln!("launchctl bootstrap failed ({e}); trying again in a second");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(e) => return Err(e),
            }
        }
        println!("Started. Logs: {}", values.log.display());
    } else {
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", "--now", "quietdeck-node.service"]))?;
        run(Command::new("systemctl").args(["--user", "restart", "quietdeck-node.service"]))?;
        println!("Started. Logs: journalctl --user -u quietdeck-node");
        println!("To keep it running while you're logged out: loginctl enable-linger \"$USER\"");
    }
    Ok(())
}

pub fn uninstall(home: &Path) -> anyhow::Result<()> {
    let file = service_file(home);
    if cfg!(target_os = "macos") {
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{}/{LABEL}", uid()?)]).output();
    } else {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", "quietdeck-node.service"]).output();
    }
    match std::fs::remove_file(&file) {
        Ok(()) => println!("Stopped and removed {}.", file.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Not installed ({} doesn't exist).", file.display())
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_fill_every_placeholder() {
        let v = Values { bin: "/opt/qd/quietdeck-node".into(), path: "/usr/bin:/bin".into(), log: "/tmp/n.log".into() };
        for t in [PLIST, UNIT] {
            let out = render(t, &v);
            assert!(!out.contains("{{"), "{out}");
            assert!(out.contains("/opt/qd/quietdeck-node"));
            assert!(out.contains("/usr/bin:/bin"));
        }
        assert!(render(PLIST, &v).contains("<string>/tmp/n.log</string>"));
    }
}
