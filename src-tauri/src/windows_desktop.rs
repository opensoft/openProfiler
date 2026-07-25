use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};

const CODEX_PACKAGE_FAMILY: &str = "OpenAI.Codex_2p2nqsd0c76g0";
const CHATGPT_PACKAGE_FAMILY: &str = "OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0";
const DEFAULT_APP_SUFFIX: &str = "!App";

#[derive(Debug, Clone)]
pub struct DesktopApp {
    pub app_user_model_id: String,
    pub installed: bool,
    pub running: bool,
}

pub fn inspect() -> DesktopApp {
    let running_processes = matching_processes().unwrap_or_default();
    let running_family = [CODEX_PACKAGE_FAMILY, CHATGPT_PACKAGE_FAMILY]
        .into_iter()
        .find(|family| {
            running_processes
                .iter()
                .any(|process| package_family_from_path(&process.path) == Some(*family))
        });
    let package_family = running_family
        .or_else(detect_installed_package)
        .unwrap_or(CODEX_PACKAGE_FAMILY);
    let app_user_model_id = env::var("OPENPROFILER_CODEX_DESKTOP_APP_ID")
        .unwrap_or_else(|_| format!("{package_family}{DEFAULT_APP_SUFFIX}"));

    DesktopApp {
        app_user_model_id,
        installed: running_family.is_some() || package_is_installed(package_family),
        running: !running_processes.is_empty(),
    }
}

pub fn stop_gracefully() -> io::Result<()> {
    let processes = matching_processes()?;
    if processes.is_empty() {
        return Ok(());
    }

    let process_ids = processes
        .iter()
        .map(|process| process.id)
        .collect::<BTreeSet<_>>();
    unsafe {
        EnumWindows(
            Some(close_window_for_process),
            (&process_ids as *const BTreeSet<u32>) as LPARAM,
        );
    }

    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if matching_processes()?.is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }

    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "The GPT desktop app is still running; quit it from the system tray and try again",
    ))
}

pub fn launch(app_user_model_id: &str) -> io::Result<()> {
    let destination = format!("shell:AppsFolder\\{app_user_model_id}");
    Command::new("explorer.exe").arg(destination).spawn()?;
    Ok(())
}

#[derive(Debug, Clone)]
struct DesktopProcess {
    id: u32,
    path: PathBuf,
}

fn matching_processes() -> io::Result<Vec<DesktopProcess>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        if let Some(path) = process_path(entry.th32ProcessID) {
            if package_family_from_path(&path).is_some() {
                processes.push(DesktopProcess {
                    id: entry.th32ProcessID,
                    path,
                });
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    Ok(processes)
}

fn process_path(process_id: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    let success =
        unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) };
    unsafe {
        CloseHandle(process);
    }
    if success == 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

fn package_family_from_path(path: &std::path::Path) -> Option<&'static str> {
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    if normalized.contains("\\windowsapps\\openai.codex_") {
        Some(CODEX_PACKAGE_FAMILY)
    } else if normalized.contains("\\windowsapps\\openai.chatgpt-desktop_") {
        Some(CHATGPT_PACKAGE_FAMILY)
    } else {
        None
    }
}

fn detect_installed_package() -> Option<&'static str> {
    [CODEX_PACKAGE_FAMILY, CHATGPT_PACKAGE_FAMILY]
        .into_iter()
        .find(|family| package_is_installed(family))
}

fn package_is_installed(package_family: &str) -> bool {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        return false;
    };
    PathBuf::from(local_app_data)
        .join("Packages")
        .join(package_family)
        .is_dir()
}

unsafe extern "system" fn close_window_for_process(window: HWND, context: LPARAM) -> i32 {
    let process_ids = unsafe { &*(context as *const BTreeSet<u32>) };
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut process_id);
    }
    if process_ids.contains(&process_id) {
        unsafe {
            PostMessageW(window, WM_CLOSE, 0, 0);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn recognizes_supported_packaged_app_paths() {
        assert_eq!(
            package_family_from_path(Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3.0_x64__2p2nqsd0c76g0\Codex.exe"
            )),
            Some(CODEX_PACKAGE_FAMILY)
        );
        assert_eq!(
            package_family_from_path(Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2.3.0_x64__2p2nqsd0c76g0\ChatGPT.exe"
            )),
            Some(CHATGPT_PACKAGE_FAMILY)
        );
    }

    #[test]
    fn ignores_unrelated_process_paths() {
        assert_eq!(
            package_family_from_path(Path::new(r"C:\Tools\codex.exe")),
            None
        );
    }
}
