use crate::{model::Paths, platform};
use anyhow::{Context, Result};
use windows::{
    core::{BSTR, VARIANT},
    Win32::System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        TaskScheduler::{
            ITaskFolder, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE,
            TASK_LOGON_INTERACTIVE_TOKEN,
        },
    },
};

struct ComScope(bool);
impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
fn with_service<T>(action: impl FnOnce(&ITaskService, &ITaskFolder) -> Result<T>) -> Result<T> {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if initialized.is_err() && initialized.0 != 0x80010106_u32 as i32 {
            initialized.ok()?;
        }
        let _scope = ComScope(initialized.is_ok());
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        let empty = VARIANT::default();
        service.Connect(&empty, &empty, &empty, &empty)?;
        let folder = service.GetFolder(&BSTR::from("\\"))?;
        action(&service, &folder)
    }
}
pub fn name(source: bool) -> &'static str {
    if source {
        "Craft Apps Manager Source Updates"
    } else {
        "Craft Apps Manager Release Updates"
    }
}
pub fn enabled(source: bool) -> bool {
    status(source).unwrap_or(false)
}
pub fn status(source: bool) -> Result<bool> {
    with_service(|_, folder| unsafe {
        match folder.GetTask(&BSTR::from(name(source))) {
            Ok(task) => Ok(task.Enabled()?.as_bool()),
            Err(error)
                if [0x80070002_u32 as i32, 0x8004130F_u32 as i32].contains(&error.code().0) =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    })
}
pub fn set(paths: &Paths, source: bool, on: bool) -> Result<()> {
    if !on {
        return with_service(|_, folder| unsafe {
            match folder.DeleteTask(&BSTR::from(name(source)), 0) {
                Ok(()) => Ok(()),
                Err(error)
                    if [0x80070002_u32 as i32, 0x8004130F_u32 as i32].contains(&error.code().0) =>
                {
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
        })
        .context("Could not disable automatic updates");
    }
    with_service(|service,folder|unsafe {
    let user=format!("{}\\{}",service.ConnectedDomain()?,service.ConnectedUser()?);
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
        r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><RegistrationInfo><Description>Craft Apps Manager automatic updates</Description></RegistrationInfo><Triggers><CalendarTrigger><Repetition><Interval>PT1H</Interval><Duration>P1D</Duration><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>{boundary}</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT1M</Delay></LogonTrigger></Triggers><Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT30M</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{args}</Arguments><WorkingDirectory>{root}</WorkingDirectory></Exec></Actions></Task>"#,
        user = platform::escape(&user),
        exe = platform::escape(&exe.display().to_string()),
        args = platform::escape(&args),
        root = platform::escape(&paths.root.display().to_string())
    );
    folder.RegisterTask(&BSTR::from(name(source)),&BSTR::from(xml),TASK_CREATE_OR_UPDATE.0,&VARIANT::from(user.as_str()),&VARIANT::default(),TASK_LOGON_INTERACTIVE_TOKEN,&VARIANT::default())?;
    Ok(())
    }).context("Could not enable automatic updates")
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_task_queries_connect_without_command_line_tools() {
        super::status(false).expect("Release task query failed");
        super::status(true).expect("Source task query failed");
    }
}
