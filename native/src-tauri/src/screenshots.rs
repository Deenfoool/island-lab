//! Local, native screenshot saving. User image pixels stay on disk.
//!
//! The frontend exports an annotated PNG and explicitly asks to save it.
//! No arbitrary file path reaches Rust: the destination is always a newly
//! created file inside the OS Pictures/Island Lab directory. No upload.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn screenshots_dir() -> Result<PathBuf, String> {
    dirs::picture_dir()
        .map(|path| path.join("Island Lab"))
        .ok_or_else(|| "Windows Pictures folder is unavailable".to_owned())
}

fn parse_png_payload(png_data: &str) -> Result<Vec<u8>, String> {
    let encoded = png_data
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "Expected PNG image data".to_owned())?;
    // Validate the encoded size before allocating a large buffer.
    if encoded.len() > MAX_PNG_BYTES.saturating_mul(4) / 3 + 8 {
        return Err("PNG screenshot exceeds 32 MiB".to_owned());
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid PNG base64 data".to_owned())?;
    if decoded.len() < 24 || decoded.len() > MAX_PNG_BYTES {
        return Err("Invalid screenshot size".to_owned());
    }
    if !decoded.starts_with(PNG_MAGIC) || &decoded[12..16] != b"IHDR" {
        return Err("Invalid PNG signature or header".to_owned());
    }
    let width = u32::from_be_bytes(decoded[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(decoded[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("Unsupported screenshot dimensions".to_owned());
    }
    Ok(decoded)
}

fn save_png_bytes(bytes: &[u8], folder: &std::path::Path) -> Result<PathBuf, String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();

    // File names are generated exclusively on the Rust side, not supplied by
    // the renderer, preventing path traversal or overwriting old screenshots.
    for _ in 0..20 {
        let counter = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(format!("Island-Lab-{time}-{counter}.png"));
        let file = OpenOptions::new().create_new(true).write(true).open(&path);
        match file {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.flush()) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(format!("Could not write screenshot: {error}"));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Cannot create a unique screenshot filename".to_owned())
}

use std::sync::{atomic::AtomicBool, Mutex};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Default)]
pub struct CaptureState {
    busy: AtomicBool,
    hotkey: AtomicBool,
    image: Mutex<Option<String>>,
}

fn show_editor(app: &AppHandle) -> Result<(), String> {
    if let Some(editor) = app.get_webview_window("editor") {
        editor.unminimize().map_err(|e| e.to_string())?;
        editor.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let result = WebviewWindowBuilder::new(app, "editor", WebviewUrl::App("index.html?mode=editor".into()))
        .title("Island Lab · Редактор")
        .inner_size(1120.0, 700.0)
        .min_inner_size(560.0, 320.0)
        .center()
        .always_on_top(true)
        .build();
    if result.is_err() {
        app.state::<CaptureState>().image.lock().unwrap().take();
    }
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_image_editor(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || show_editor(&app))
        .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn take_screenshot(window: tauri::WebviewWindow, state: tauri::State<'_, CaptureState>) -> Option<String> {
    if window.label() != "editor" {
        return None;
    }
    state.image.lock().unwrap().take()
}

#[tauri::command]
pub fn print_screen_available(state: tauri::State<'_, CaptureState>) -> bool {
    state.hotkey.load(Ordering::Relaxed)
}

#[tauri::command]
pub async fn capture_monitor_native(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || capture_and_open(&app, false))
        .await.map_err(|e| e.to_string())?
}

fn capture_and_open(app: &AppHandle, cursor_monitor: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

        let state = app.state::<CaptureState>();
        if state.busy.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let result = (|| {
            // Preserve unsaved work when Print Screen is pressed again.
            if app.get_webview_window("editor").is_some() {
                return show_editor(app);
            }
            let main = app.get_webview_window("main").ok_or("Окно Island Lab закрыто")?;
            let mut cursor = POINT { x: 0, y: 0 };
            let monitor = if cursor_monitor && unsafe { GetCursorPos(&mut cursor) } != 0 {
                main.monitor_from_point(cursor.x as f64, cursor.y as f64)
            } else {
                main.current_monitor()
            }.map_err(|e| e.to_string())?.ok_or("Не удалось определить монитор")?;
            let position = *monitor.position();
            let size = *monitor.size();

            let hide_main = !cursor_monitor && main.is_visible().unwrap_or(false);
            if hide_main {
                main.hide().map_err(|e| e.to_string())?;
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            let image = capture_monitor(position.x, position.y, size.width, size.height);
            if hide_main {
                main.show().map_err(|e| e.to_string())?;
            }
            *state.image.lock().unwrap() = Some(image?);
            show_editor(app)
        })();
        state.busy.store(false, Ordering::Release);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (app, cursor_monitor);
        Err("Захват монитора доступен в Windows".into())
    }
}

#[cfg(windows)]
mod print_screen {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };

    const CAPTURE: u32 = WM_APP + 1;
    static PRESSED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn keyboard(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && data != 0
            && (*(data as *const KBDLLHOOKSTRUCT)).vkCode == u32::from(VK_SNAPSHOT)
        {
            match message as u32 {
                WM_KEYDOWN if [VK_MENU, VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
                    .into_iter().all(|key| GetAsyncKeyState(i32::from(key)) >= 0) =>
                {
                    if !PRESSED.swap(true, Ordering::Relaxed)
                        && PostThreadMessageW(GetCurrentThreadId(), CAPTURE, 0, 0) == 0
                    {
                        PRESSED.store(false, Ordering::Relaxed);
                        return CallNextHookEx(null_mut(), code, message, data);
                    }
                    return 1;
                }
                WM_KEYUP if PRESSED.swap(false, Ordering::Relaxed) => return 1,
                _ => {}
            }
        }
        CallNextHookEx(null_mut(), code, message, data)
    }

    pub fn run(app: AppHandle) {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(std::ptr::null()), 0);
            if hook.is_null() {
                return;
            }
            app.state::<CaptureState>().hotkey.store(true, Ordering::Relaxed);
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                if message.message == CAPTURE {
                    let app = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = capture_and_open(&app, true) {
                            use tauri::Emitter;
                            let _ = app.emit_to("main", "capture-error", error);
                        }
                    });
                }
            }
            UnhookWindowsHookEx(hook);
        }
    }
}

pub fn start_print_screen(app: AppHandle) {
    #[cfg(windows)]
    let _ = std::thread::Builder::new().name("island-print-screen".into()).spawn(move || print_screen::run(app));
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
fn capture_monitor(x: i32, y: i32, width: u32, height: u32) -> Result<String, String> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::*;

    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 24_000_000 {
        return Err("Разрешение монитора превышает лимит захвата".into());
    }

    struct Capture {
        screen: HDC,
        memory: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            unsafe {
                if !self.previous.is_null() {
                    SelectObject(self.memory, self.previous);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                if !self.memory.is_null() {
                    DeleteDC(self.memory);
                }
                if !self.screen.is_null() {
                    ReleaseDC(null_mut(), self.screen);
                }
            }
        }
    }

    let rgb = unsafe {
        let mut capture = Capture {
            screen: GetDC(null_mut()), memory: null_mut(), bitmap: null_mut(), previous: null_mut(),
        };
        if capture.screen.is_null() {
            return Err("Windows не предоставила поверхность экрана".into());
        }
        capture.memory = CreateCompatibleDC(capture.screen);
        if capture.memory.is_null() {
            return Err("Не удалось подготовить буфер экрана".into());
        }

        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        info.bmiHeader.biHeight = -(height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = null_mut();
        capture.bitmap = CreateDIBSection(capture.memory, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if capture.bitmap.is_null() || pixels.is_null() {
            return Err("Не удалось выделить память для снимка".into());
        }
        let previous = SelectObject(capture.memory, capture.bitmap);
        if previous.is_null() || previous as isize == -1 {
            return Err("Не удалось подключить буфер снимка".into());
        }
        capture.previous = previous;
        if BitBlt(capture.memory, 0, 0, width as i32, height as i32, capture.screen, x, y, SRCCOPY | CAPTUREBLT) == 0 {
            return Err("Windows не смогла захватить монитор".into());
        }
        GdiFlush();
        let bytes = std::slice::from_raw_parts(pixels.cast::<u8>(), (width * height * 4) as usize);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for bgra in bytes.chunks_exact(4) {
            rgb.extend_from_slice(&[bgra[2], bgra[1], bgra[0]]);
        }
        rgb
    };
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer.write_image_data(&rgb).map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

#[tauri::command(async)]
pub fn save_screenshot_native(png_data: String) -> Result<String, String> {
    #[cfg(windows)]
    {
        let bytes = parse_png_payload(&png_data)?;
        let path = save_png_bytes(&bytes, &screenshots_dir()?)?;
        Ok(path.to_string_lossy().into_owned())
    }
    #[cfg(not(windows))]
    {
        let _ = png_data;
        Err("Native screenshot saving requires Windows".to_owned())
    }
}

#[tauri::command]
pub fn open_screenshot_folder_native() -> Result<String, String> {
    #[cfg(windows)]
    {
        let path = screenshots_dir()?;
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        std::process::Command::new("explorer.exe")
            .arg(&path)
            .spawn()
            .map_err(|error| format!("Could not open Explorer: {error}"))?;
        Ok(path.to_string_lossy().into_owned())
    }
    #[cfg(not(windows))]
    {
        Err("Windows Explorer is unavailable".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_png_payload, PNG_MAGIC, MAX_PNG_BYTES};
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    fn make_data(width: u32, height: u32) -> String {
        let mut bytes = Vec::from(PNG_MAGIC.as_slice());
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        format!("data:image/png;base64,{}", STANDARD.encode(bytes))
    }

    #[test]
    fn accepts_png_header_with_valid_dimensions() {
        assert_eq!(&parse_png_payload(&make_data(1920, 1080)).unwrap()[..8], PNG_MAGIC);
    }

    #[test]
    fn rejects_invalid_types_and_dimensions() {
        assert!(parse_png_payload("data:image/jpeg;base64,AAAA").is_err());
        assert!(parse_png_payload(&make_data(0, 1080)).is_err());
        assert!(parse_png_payload(&make_data(8193, 1080)).is_err());
        assert!(parse_png_payload("data:image/png;base64,garbage").is_err());
    }

    #[test]
    fn limits_input_before_allocating() {
        let huge = "A".repeat(MAX_PNG_BYTES * 4 / 3 + 20);
        assert!(parse_png_payload(&format!("data:image/png;base64,{huge}")).is_err());
    }

    #[test]
    fn creates_files_without_overwriting() {
        let dir = std::env::temp_dir().join(format!(
            "island-lab-screenshot-test-{}-{}",
            std::process::id(),
            super::FILE_COUNTER.fetch_add(1, super::Ordering::Relaxed)
        ));
        let first = super::save_png_bytes(b"test1", &dir).unwrap();
        let second = super::save_png_bytes(b"test2", &dir).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"test1");
        assert_eq!(std::fs::read(second).unwrap(), b"test2");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
