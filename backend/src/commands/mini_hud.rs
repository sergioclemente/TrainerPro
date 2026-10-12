//! The mini HUD: a small transparent always-on-top window that shows the
//! ride's five live fields while another app has the screen. It loads the
//! same frontend bundle with `?view=hud`, so it consumes the same Tauri
//! events as the Player and sends the same player commands.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::app_error::AppError;

type R<T> = Result<T, AppError>;

/// Window label of the HUD; the capability file grants it the same
/// permissions as the main window.
pub const HUD_WINDOW_LABEL: &str = "hud";
const MAIN_WINDOW_LABEL: &str = "main";
/// Frontend route the HUD window loads; `main.tsx` switches on it.
const HUD_ROUTE: &str = "index.html?view=hud";
/// Logical size of the HUD, matching the design's 400×100 strip.
const HUD_WIDTH_PX: f64 = 400.0;
const HUD_HEIGHT_PX: f64 = 100.0;
/// Gap from the screen's top-right corner on first open.
const HUD_SCREEN_MARGIN_PX: f64 = 24.0;

fn window_error(e: tauri::Error) -> AppError {
    AppError::new("window", e.to_string())
}

/// Show the HUD, creating it on first use. The main window stays as it is:
/// hiding it would also stop controller and voice input, which key off the
/// main window being active.
#[tauri::command]
pub async fn open_mini_hud(app: AppHandle) -> R<()> {
    match app.get_webview_window(HUD_WINDOW_LABEL) {
        Some(hud) => {
            hud.show().map_err(window_error)?;
            hud.set_focus().map_err(window_error)?;
        }
        None => {
            let mut builder = WebviewWindowBuilder::new(
                &app,
                HUD_WINDOW_LABEL,
                WebviewUrl::App(HUD_ROUTE.into()),
            )
            .title("TrainerPro HUD")
            .inner_size(HUD_WIDTH_PX, HUD_HEIGHT_PX)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true);
            if let Some((x, y)) = top_right_position(&app) {
                builder = builder.position(x, y);
            }
            builder.build().map_err(window_error)?;
        }
    }
    Ok(())
}

/// Close the HUD and bring the main window back.
#[tauri::command]
pub async fn close_mini_hud(app: AppHandle) -> R<()> {
    if let Some(hud) = app.get_webview_window(HUD_WINDOW_LABEL) {
        hud.close().map_err(window_error)?;
    }
    if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        main.unminimize().map_err(window_error)?;
        main.show().map_err(window_error)?;
        main.set_focus().map_err(window_error)?;
    }
    Ok(())
}

/// Logical position that parks the HUD in the primary screen's top-right
/// corner; `None` leaves the placement to the window manager.
fn top_right_position(app: &AppHandle) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let size = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    Some((
        origin.x + size.width - HUD_WIDTH_PX - HUD_SCREEN_MARGIN_PX,
        origin.y + HUD_SCREEN_MARGIN_PX,
    ))
}
