#[path = "island_protocol.rs"]
mod protocol;

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, WebviewWindow};

const EXE_NAMES: [&str; 2] = ["Snowy New Year.exe", "snowy-new-year.exe"];

fn saved_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("snowy-executable.txt"))
        .map_err(|error| error.to_string())
}

fn is_snowy_exe(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| EXE_NAMES.iter().any(|exe| name.eq_ignore_ascii_case(exe)))
        && path.is_file()
}

fn exe_in(dir: &Path) -> Option<PathBuf> {
    EXE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

fn extracted_exe_in(dir: &Path) -> Option<PathBuf> {
    exe_in(dir).or_else(|| {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .find_map(|entry| exe_in(&entry.path()))
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnowStatus {
    connected: bool,
    incompatible: bool,
    can_launch: bool,
    falling: bool,
    caps: bool,
    snowballs: bool,
    intensity: u8,
}

fn companion(app: &AppHandle) -> Option<PathBuf> {
    let saved = saved_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && is_snowy_exe(path));
    saved
        .or_else(|| exe_in(std::env::current_exe().ok()?.parent()?))
        .or_else(|| {
            ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"]
                .into_iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from)
                .find_map(|dir| {
                    exe_in(&dir.join("Snowy New Year"))
                        .or_else(|| exe_in(&dir.join("Programs").join("Snowy New Year")))
                })
        })
        .or_else(|| {
            [dirs::download_dir(), dirs::desktop_dir()]
                .into_iter()
                .flatten()
                .find_map(|dir| extracted_exe_in(&dir))
        })
}

#[cfg(windows)]
fn already_running() -> bool {
    use windows_sys::{
        core::w,
        Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE},
        },
    };
    let mutex = unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            0,
            w!("Local\\SnowyNewYear.Rust"),
        )
    };
    if mutex.is_null() {
        return false;
    }
    unsafe { CloseHandle(mutex) };
    true
}

#[cfg(not(windows))]
fn already_running() -> bool {
    false
}

#[cfg(windows)]
fn request(field: usize, value: isize) -> Result<Option<usize>, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, RegisterWindowMessageW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK,
    };
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<_>>();
    // Only the named Snowy New Year window can receive these versioned integer messages.
    let hwnd = unsafe { FindWindowW(wide(protocol::CONTROLLER).as_ptr(), std::ptr::null()) };
    if hwnd.is_null() {
        return Ok(None);
    }
    let message = unsafe { RegisterWindowMessageW(wide(protocol::MESSAGE).as_ptr()) };
    let mut reply = 0;
    if message == 0
        || unsafe {
            SendMessageTimeoutW(
                hwnd,
                message,
                field,
                value,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                300,
                &mut reply,
            )
        } == 0
    {
        return Err("Snowy New Year не отвечает".into());
    }
    Ok(Some(reply))
}

#[cfg(not(windows))]
fn request(_: usize, _: isize) -> Result<Option<usize>, String> {
    Ok(None)
}

#[tauri::command]
pub async fn snow_status(app: AppHandle) -> Result<SnowStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let reply = request(0, 0)?;
        let bits = reply.unwrap_or(0);
        let connected = bits & !0xffff == protocol::ACK;
        let incompatible = !connected && (reply.is_some() || already_running());
        Ok(SnowStatus {
            connected,
            incompatible,
            can_launch: !connected && !incompatible && companion(&app).is_some(),
            falling: connected && bits & 1 != 0,
            caps: connected && bits & 2 != 0,
            snowballs: connected && bits & 4 != 0,
            intensity: if connected {
                ((bits >> 8) & 0xff) as u8
            } else {
                0
            },
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn snow_set(field: usize, value: isize) -> Result<(), String> {
    if protocol::Field::decode(field, value).is_none() {
        return Err("Недопустимое значение настройки".into());
    }
    tauri::async_runtime::spawn_blocking(move || match request(field, value)? {
        Some(protocol::ACK) => Ok(()),
        _ => Err("Подключение к Snowy New Year потеряно".into()),
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn snow_launch(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if request(0, 0)?.is_some() || already_running() {
            return Ok(());
        }
        let exe = companion(&app).ok_or("Файл Snowy New Year.exe не найден")?;
        let directory = exe
            .parent()
            .ok_or("Не удалось определить папку Snowy New Year")?;
        std::process::Command::new(&exe)
            .current_dir(directory)
            .arg("--tray")
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn snow_locate(window: WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::{mem::size_of, os::windows::ffi::OsStringExt};
        use windows_sys::{
            core::w,
            Win32::UI::Controls::Dialogs::{
                CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
                OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
            },
        };
        let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        let saved = saved_path(window.app_handle())?;
        tauri::async_runtime::spawn_blocking(move || {
            let mut filename = vec![0u16; 32_768];
            let mut dialog = OPENFILENAMEW {
                lStructSize: size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: owner as _,
                lpstrFile: filename.as_mut_ptr(),
                nMaxFile: filename.len() as u32,
                lpstrTitle: w!("Выберите Snowy New Year.exe"),
                lpstrFilter: w!("Snowy New Year (*.exe)\0*.exe\0"),
                Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
                ..Default::default()
            };
            if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
                return if unsafe { CommDlgExtendedError() } == 0 {
                    Ok(())
                } else {
                    Err("Не удалось открыть выбор файла".into())
                };
            }
            let len = filename
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(filename.len());
            let path = PathBuf::from(std::ffi::OsString::from_wide(&filename[..len]));
            if !is_snowy_exe(&path) {
                return Err("Выберите распакованный файл Snowy New Year.exe".into());
            }
            let dir = saved
                .parent()
                .ok_or("Не удалось определить папку настроек Island Lab")?;
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
            std::fs::write(
                &saved,
                path.to_str().ok_or("Не удалось прочитать путь к файлу")?,
            )
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("Выбор Snowy New Year доступен в Windows".into())
    }
}

#[tauri::command]
pub async fn snow_open_release(window: WebviewWindow, bundle: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::ptr::null;
        use windows_sys::{
            core::w,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize;
        tauri::async_runtime::spawn_blocking(move || {
            let url = if bundle {
                w!("https://github.com/Deenfoool/island-lab/releases/latest")
            } else {
                w!("https://github.com/Deenfoool/snowy-New-Year/releases/latest")
            };
            let result = unsafe {
                ShellExecuteW(owner as _, w!("open"), url, null(), null(), SW_SHOWNORMAL)
            };
            if result as isize > 32 {
                Ok(())
            } else {
                Err("Не удалось открыть GitHub в браузере".into())
            }
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, bundle);
        Err("Открытие релиза доступно в Windows".into())
    }
}

#[tauri::command]
pub fn snow_edge(window: WebviewWindow, insets: Option<[f64; 4]>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{
            Accessibility::NotifyWinEvent,
            WindowsAndMessaging::{GetPropW, SetPropW, EVENT_OBJECT_LOCATIONCHANGE, OBJID_WINDOW},
        };
        let scale = window.scale_factor().map_err(|error| error.to_string())?;
        let mut packed = protocol::SNOW_EDGE_HIDDEN;
        if let Some(insets) = insets {
            let pixels = insets.map(|value| (value * scale).ceil());
            if pixels.iter().all(|value| (0.0..=254.0).contains(value)) {
                packed = u32::from_le_bytes(pixels.map(|value| value as u8));
            }
        }
        let hwnd = window.hwnd().map_err(|error| error.to_string())?.0;
        let name: Vec<u16> = protocol::SNOW_EDGE_PROPERTY
            .encode_utf16()
            .chain([0])
            .collect();
        let value = packed as usize as *mut std::ffi::c_void;
        unsafe {
            if GetPropW(hwnd, name.as_ptr()) != value {
                if SetPropW(hwnd, name.as_ptr(), value) == 0 {
                    return Err("Не удалось опубликовать край Island Lab".into());
                }
                NotifyWinEvent(EVENT_OBJECT_LOCATIONCHANGE, hwnd, OBJID_WINDOW, 0);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (window, insets);
    Ok(())
}
