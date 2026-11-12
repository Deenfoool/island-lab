#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
mod media;
mod audio;
mod desktop;
mod screenshots;
mod snow;

use tauri::{Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Default)]
struct WindowPositionState {
    initialized: AtomicBool,
}

fn clamp_inside_screen(value: i32, screen_start: i32, screen_size: i32, window_size: i32) -> i32 {
    // Never overflow on a very small monitor or high display scaling.
    let margin = 8;
    let first = screen_start.saturating_add(margin);
    let last = screen_start
        .saturating_add(screen_size)
        .saturating_sub(window_size)
        .saturating_sub(margin);
    value.clamp(first.min(last), first.max(last))
}

/// Experimental ordinary Windows window; desktop-only docking is not enabled.
///
/// On first startup place the capsule at the top center of the active monitor.
/// Later expansions keep the horizontal center and top edge of the *moved*
/// capsule instead of unexpectedly jumping back to the monitor center.
/// The native viewport remains small when the island is collapsed.
#[tauri::command]
fn set_island_expanded(
    window: WebviewWindow,
    position: State<'_, WindowPositionState>,
    expanded: bool,
    wave_room: Option<bool>,
) -> Result<(), String> {
    let (requested_width, requested_height): (f64, f64) = if expanded {
        (940.0_f64, if wave_room.unwrap_or(false) { 380.0 } else { 320.0 })
    } else {
        (360.0_f64, if wave_room.unwrap_or(false) { 104.0 } else { 75.0 })
    };

    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let old_position = window.outer_position().map_err(|e| e.to_string())?;
    let old_size = window.outer_size().map_err(|e| e.to_string())?;

    let (mut width, mut height) = (requested_width, requested_height);
    if let Some(m) = &monitor {
        width = width.min((m.size().width as f64 - 16.0) / scale).max(1.0);
        height = height.min((m.size().height as f64 - 36.0) / scale).max(1.0);
    }

    let new_width_px = (width * scale).round() as i32;
    let new_height_px = (height * scale).round() as i32;
    let first = !position.initialized.swap(true, Ordering::SeqCst);
    let (desired_x, desired_y) = if first {
        if let Some(m) = &monitor {
            (
                m.position().x
                    + (m.size().width as i32 - new_width_px) / 2,
                m.position().y + (20.0 * scale).round() as i32,
            )
        } else {
            (old_position.x, old_position.y)
        }
    } else {
        (
            old_position.x + (old_size.width as i32 - new_width_px) / 2,
            old_position.y,
        )
    };

    let (x, y) = if let Some(m) = &monitor {
        (
            clamp_inside_screen(
                desired_x,
                m.position().x,
                m.size().width as i32,
                new_width_px,
            ),
            clamp_inside_screen(
                desired_y,
                m.position().y,
                m.size().height as i32,
                new_height_px,
            ),
        )
    } else {
        (desired_x, desired_y)
    };

    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn quit_island_lab(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn open_preferences(app: tauri::AppHandle, section: String) -> Result<(), String> {
    let section = match section.as_str() {
        "music" => "music",
        "weather" => "weather",
        "system" => "system",
        "diagnostics" => "diagnostics",
        _ => "general",
    };
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(window) = app.get_webview_window("preferences") {
            window.unminimize().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            return window.emit("preferences-section", section).map_err(|e| e.to_string());
        }
        let url = format!("index.html?mode=preferences&section={section}");
        WebviewWindowBuilder::new(&app, "preferences", WebviewUrl::App(url.into()))
            .title("Island Lab · Настройки")
            .inner_size(770.0, 500.0)
            .min_inner_size(560.0, 380.0)
            .center()
            .build().map(|_| ()).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .manage(WindowPositionState::default())
        .manage(screenshots::CaptureState::default())
        .setup(|app| {
            screenshots::start_print_screen(app.handle().clone());
            Ok(())
        })
        .manage(audio::AudioCaptureState::default())
        .manage(desktop::DesktopModeState::default())
        .invoke_handler(tauri::generate_handler![
            set_island_expanded,
            open_preferences,
            quit_island_lab,
            media::system_media_snapshot,
            media::system_media_action,
            audio::audio_capture_start,
            audio::audio_capture_stop,
            desktop::desktop_mode_status,
            desktop::desktop_mode_try,
            desktop::desktop_mode_keep,
            desktop::desktop_mode_cancel,
            snow::snow_status,
            snow::snow_edge,
            snow::snow_set,
            snow::snow_launch,
            snow::snow_locate,
            snow::snow_open_release,
            screenshots::capture_monitor_native,
            screenshots::open_image_editor,
            screenshots::take_screenshot,
            screenshots::print_screen_available,
            screenshots::save_screenshot_native,
            screenshots::open_screenshot_folder_native
        ])
        .run(tauri::generate_context!())
        .expect("Island Lab native host could not start");
}

#[cfg(test)]
mod tests {
    use super::clamp_inside_screen;

    #[test]
    fn position_within_screen_edges() {
        assert_eq!(clamp_inside_screen(-400, 0, 1920, 360), 8);
        assert_eq!(clamp_inside_screen(4000, 0, 1920, 360), 1552);
        assert_eq!(clamp_inside_screen(580, 300, 1200, 460), 580);
    }

    #[test]
    fn negative_coordinates_are_supported_for_secondary_displays() {
        assert_eq!(clamp_inside_screen(-2000, -1920, 1920, 360), -1912);
        assert_eq!(clamp_inside_screen(-100, -1920, 1920, 360), -368);
    }
}
