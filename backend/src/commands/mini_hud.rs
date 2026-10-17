//! The mini HUD: a small transparent always-on-top window that shows the
//! ride's five live fields while another app has the screen. It loads the
//! same frontend bundle with `?view=hud`, so it consumes the same Tauri
//! events as the Player and sends the same player commands.
//!
//! The HUD is a mode of the app, not a second app: opening it hides the
//! main window, closing it brings the main window back, and the two windows
//! close together.

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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

/// Show the HUD, creating it on first use, and hide the main window. The
/// HUD takes over controller pause/resume while the main window is hidden;
/// voice capture stays with the main window and is unavailable meanwhile.
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
            let hud = builder.build().map_err(window_error)?;
            // Closed by any other route (Cmd+W, the OS): the main window
            // must come back or the app would seem to have vanished.
            let handle = app.clone();
            hud.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    show_main_window(&handle);
                }
            });
        }
    }
    if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        main.hide().map_err(window_error)?;
    }
    Ok(())
}

/// Close the HUD and bring the main window back.
#[tauri::command]
pub async fn close_mini_hud(app: AppHandle) -> R<()> {
    if let Some(hud) = app.get_webview_window(HUD_WINDOW_LABEL) {
        hud.close().map_err(window_error)?;
    }
    show_main_window(&app);
    Ok(())
}

/// Hide the HUD without bringing the main window back: the rider wants
/// TrainerPro out of the way entirely. The Dock icon restores the app.
#[tauri::command]
pub async fn hide_mini_hud(app: AppHandle) -> R<()> {
    if let Some(hud) = app.get_webview_window(HUD_WINDOW_LABEL) {
        hud.hide().map_err(window_error)?;
    }
    Ok(())
}

/// Keep the two windows one app; wire this at startup. Closing the main
/// window takes the HUD with it, and the main window coming back by any
/// route (Dock thumbnail, Mission Control) dismisses the HUD.
pub fn couple_windows(app: &AppHandle) {
    let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let handle = app.clone();
    main.on_window_event(move |event| match event {
        WindowEvent::Destroyed | WindowEvent::Focused(true) => close_hud(&handle),
        _ => {}
    });
}

/// Clicking the Dock icon means "show me TrainerPro": the full window, with
/// the HUD dismissed, whether the HUD was showing or hidden.
pub fn on_run_event(app: &AppHandle, event: &RunEvent) {
    if let RunEvent::Reopen { .. } = event {
        if app.get_webview_window(HUD_WINDOW_LABEL).is_some() {
            close_hud(app);
            show_main_window(app);
        }
    }
}

fn close_hud(app: &AppHandle) {
    if let Some(hud) = app.get_webview_window(HUD_WINDOW_LABEL) {
        let _ = hud.close();
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
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
