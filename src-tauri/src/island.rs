use crate::runtime::{Event, Runtime};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};
use tauri::{Emitter, LogicalSize, Manager, PhysicalPosition};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

#[derive(Default)]
pub struct Island {
    pub active: AtomicBool,
    pinned: AtomicBool,
    escape_owned: AtomicBool,
    revision: Arc<AtomicU64>,
}

fn layout(app: &tauri::AppHandle, expanded: bool) {
    let Some(window) = app.get_webview_window("island") else {
        return;
    };
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let width =
            (if expanded { 620f64 } else { 420f64 }).min(area.size.width as f64 / scale - 16.0);
        let height =
            (if expanded { 560f64 } else { 98f64 }).min(area.size.height as f64 / scale - 40.0);
        let _ = window.set_size(LogicalSize::new(width, height));
        let _ = window.set_position(PhysicalPosition::new(
            area.position.x + (area.size.width as i32 - (width * scale) as i32) / 2,
            area.position.y + (12.0 * scale) as i32,
        ));
    }
    let _ = window.set_ignore_cursor_events(false);
    let _ = window.show();
}

pub fn hide(app: &tauri::AppHandle) {
    let state = app.state::<Island>();
    state.revision.fetch_add(1, SeqCst);
    state.active.store(false, SeqCst);
    state.pinned.store(false, SeqCst);
    if state.escape_owned.swap(false, SeqCst) {
        let _ = app.global_shortcut().unregister("Escape");
    }
    if let Some(window) = app.get_webview_window("island") {
        let _ = window.hide();
    }
}

pub fn dismiss(app: &tauri::AppHandle) {
    hide(app);
    if let Some(runtime) = app.try_state::<Runtime>() {
        runtime.shared.epoch.fetch_add(1, SeqCst);
        runtime.shared.activate.store(false, SeqCst);
        runtime.shared.dictating.store(false, SeqCst);
        runtime.shared.emit("deactivated", "");
    }
}

pub fn expand(app: &tauri::AppHandle, expanded: bool) {
    let state = app.state::<Island>();
    if !state.active.load(SeqCst) {
        return;
    }
    state.pinned.store(expanded, SeqCst);
    state.revision.fetch_add(1, SeqCst);
    layout(app, expanded);
    let _ = app.emit_to("island", "island-layout", expanded);
}

pub fn update(app: &tauri::AppHandle, event: &Event) {
    if event.kind == "level" {
        return;
    }
    let state = app.state::<Island>();
    let enabled = app
        .try_state::<Runtime>()
        .is_some_and(|r| r.shared.snapshot().island_enabled);
    if !enabled
        || event.kind == "deactivated"
        || (event.kind == "microphone" && event.text == "muted")
    {
        hide(app);
        return;
    }
    // Only an explicit wake/key activation opens the island. Logs and typed requests do not.
    if matches!(event.kind.as_str(), "activated" | "wake_preview") {
        if let Some(runtime) = app.try_state::<Runtime>() {
            let s = runtime.shared.snapshot();
            let _ = app.emit_to(
                "island",
                "island-style",
                (s.island_bg, s.island_edge, s.island_mic, s.island_flat),
            );
        }
        layout(app, false);
        state.revision.fetch_add(1, SeqCst);
        state.active.store(true, SeqCst);
        state.pinned.store(false, SeqCst);
        if !app.global_shortcut().is_registered("Escape")
            && app.global_shortcut().register("Escape").is_ok()
        {
            state.escape_owned.store(true, SeqCst);
        }

        let _ = app.emit_to("island", "island-layout", false);
        return;
    }
    if !state.active.load(SeqCst) {
        return;
    }
    if event.kind == "answer" || event.kind == "preview_result" {
        expand(app, true);
        return;
    }
    if state.pinned.load(SeqCst) {
        return;
    }
    let idle = event.kind == "status"
        && (event.text.starts_with("Ожидаю") || event.text == "Диктовка завершена");
    if idle || matches!(event.kind.as_str(), "info" | "error") {
        let revision = state.revision.fetch_add(1, SeqCst) + 1;
        let counter = state.revision.clone();
        let app = app.clone();
        let delay = if event.kind == "error" { 8000 } else { 4000 };
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            if counter.load(SeqCst) == revision {
                hide(&app);
            }
        });
    } else if event.kind == "status" {
        state.revision.fetch_add(1, SeqCst);
    }
}
