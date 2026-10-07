use crate::{model::Paths, platform};
use anyhow::{bail, Result};
use std::{fs, process::Command};
pub fn name(source: bool) -> &'static str {
    if source {
        "Craft Apps Rust Source Updates"
    } else {
        "Craft Apps Rust Release Updates"
    }
}
pub fn enabled(source: bool) -> bool {
    platform::output(Command::new("schtasks.exe").args(["/Query", "/TN", name(source), "/XML"]))
        .ok()
        .is_some_and(|o| {
            o.status.success()
                && !String::from_utf8_lossy(&o.stdout).contains("<Enabled>false</Enabled>")
        })
}
pub fn set(paths: &Paths, source: bool, on: bool) -> Result<()> {
    if !on {
        if !enabled(source) {
            return Ok(());
        }
        let o = platform::output(Command::new("schtasks.exe").args([
            "/Delete",
            "/TN",
            name(source),
            "/F",
        ]))?;
        if !o.status.success() {
            bail!(
                "Could not disable automatic updates: {}",
                String::from_utf8_lossy(&o.stderr)
            )
        }
        return Ok(());
    }
    let user = platform::output(&mut Command::new("whoami.exe"))?;
    let user = String::from_utf8_lossy(&user.stdout).trim().to_string();
    let exe = std::env::current_exe()?;
    let args = format!(
        "{} --root \"{}\" --tools \"{}\" --background",
        if source {
            "--update-source"
        } else {
            "--update"
        },
        paths.root.display(),
        paths.tools.display()
    );
    let boundary = (chrono::Local::now() + chrono::Duration::hours(1)).format("%Y-%m-%dT%H:%M:%S");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><RegistrationInfo><Description>Craft Apps Updater automatic updates</Description></RegistrationInfo><Triggers><CalendarTrigger><Repetition><Interval>PT1H</Interval><Duration>P1D</Duration><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>{boundary}</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT1M</Delay></LogonTrigger></Triggers><Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT30M</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{args}</Arguments><WorkingDirectory>{root}</WorkingDirectory></Exec></Actions></Task>"#,
        user = platform::escape(&user),
        exe = platform::escape(&exe.display().to_string()),
        args = platform::escape(&args),
        root = platform::escape(&paths.root.display().to_string())
    );
    let path = paths.at("runtime/task.xml");
    fs::create_dir_all(path.parent().unwrap())?;
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&path, bytes)?;
    let out = platform::output(
        Command::new("schtasks.exe")
            .args(["/Create", "/TN", name(source), "/XML"])
            .arg(&path)
            .arg("/F"),
    )?;
    let _ = fs::remove_file(path);
    if !out.status.success() {
        bail!(
            "Could not enable automatic updates: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
    Ok(())
}
