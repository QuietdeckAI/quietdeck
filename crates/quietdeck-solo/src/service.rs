//! `quietdeck-solo install` and `uninstall`: solo as a launchd agent (macOS) or a systemd user service (Linux), the
//! way `quietdeck-node install` does it for the node. The templates are deploy/com.quietdeck.solo.plist and
//! deploy/quietdeck-solo.service.

use std::path::{Path, PathBuf};
use std::process::Command;

const PLIST: &str = include_str!("../../../deploy/com.quietdeck.solo.plist");
const UNIT: &str = include_str!("../../../deploy/quietdeck-solo.service");
const LABEL: &str = "com.quietdeck.solo";
const UNIT_NAME: &str = "quietdeck-solo.service";

pub struct Values {
    pub bin: PathBuf,
    pub config: PathBuf,
    pub path: String,
    pub log: PathBuf,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn render(template: &str, v: &Values, plist: bool) -> String {
    let esc = |s: String| if plist { xml(&s) } else { s };
    template
        .replace("{{BIN}}", &esc(v.bin.display().to_string()))
        .replace("{{CONFIG}}", &esc(v.config.display().to_string()))
        .replace("{{PATH}}", &esc(v.path.clone()))
        .replace("{{LOG}}", &esc(v.log.display().to_string()))
}

fn service_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
    } else {
        home.join(".config/systemd/user").join(UNIT_NAME)
    }
}

pub fn log_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Logs/quietdeck-solo.log")
    } else {
        home.join(".local/state/quietdeck/solo.log")
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

/// Write the service file and start it. `config` has already been checked, so the service doesn't restart forever on
/// a bad one.
pub fn install(home: &Path, config: &Path, start: bool) -> anyhow::Result<()> {
    let values = Values {
        bin: std::env::current_exe()?.canonicalize()?,
        config: config.canonicalize()?,
        path: std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        log: log_file(home),
    };
    if let Some(dir) = values.log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = service_file(home);
    std::fs::create_dir_all(file.parent().expect("a service file has a directory"))?;
    let mac = cfg!(target_os = "macos");
    std::fs::write(&file, render(if mac { PLIST } else { UNIT }, &values, mac))?;
    println!("Wrote {}.", file.display());
    if !start {
        return Ok(());
    }
    if mac {
        let domain = format!("gui/{}", uid()?);
        let target = format!("{domain}/{LABEL}");
        // As for the node: bootout returns before the old job is gone, and a bootstrap in that gap fails with
        // "5: Input/output error", so wait for it, then try a few times.
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
        run(Command::new("systemctl").args(["--user", "enable", "--now", UNIT_NAME]))?;
        run(Command::new("systemctl").args(["--user", "restart", UNIT_NAME]))?;
        println!("Started. Logs: journalctl --user -u quietdeck-solo");
        println!("To keep it running while you're logged out: loginctl enable-linger \"$USER\"");
    }
    Ok(())
}

pub fn uninstall(home: &Path) -> anyhow::Result<()> {
    let file = service_file(home);
    if cfg!(target_os = "macos") {
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{}/{LABEL}", uid()?)]).output();
    } else {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", UNIT_NAME]).output();
    }
    match std::fs::remove_file(&file) {
        Ok(()) => println!("Stopped and removed {}. Your settings and data are still there.", file.display()),
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
    fn templates_fill_every_placeholder_and_quote_paths_with_spaces() {
        let v = Values {
            bin: "/Users/a b/bin/quietdeck-solo".into(),
            config: "/Users/a b/.config/quietdeck/solo.yaml".into(),
            path: "/opt/homebrew/bin:/usr/bin".into(),
            log: "/tmp/s.log".into(),
        };
        let unit = render(UNIT, &v, false);
        assert!(!unit.contains("{{"), "{unit}");
        assert!(unit.contains(
            r#"ExecStart="/Users/a b/bin/quietdeck-solo" --config "/Users/a b/.config/quietdeck/solo.yaml" run"#
        ));
        let plist = render(PLIST, &v, true);
        assert!(!plist.contains("{{"));
        assert!(plist.contains("<string>/Users/a b/.config/quietdeck/solo.yaml</string>"));
        assert!(plist.contains("<string>/tmp/s.log</string>"));
        let odd = Values { bin: "/x/a&b".into(), ..v };
        assert!(render(PLIST, &odd, true).contains("/x/a&amp;b"), "XML-escaped");
    }
}
