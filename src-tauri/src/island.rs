use crate::runtime::{Event, Runtime};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};
use tauri::{Manager, PhysicalPosition};

#[derive(Default)]
pub struct Island {
    active: AtomicBool,
    revision: Arc<AtomicU64>,
}

pub fn update(app: &tauri::AppHandle, event: &Event) {
    if event.kind == "level" {
        return;
    }
    let Some(window) = app.get_webview_window("island") else {
        return;
    };
    let state = app.state::<Island>();
    let enabled = app
        .try_state::<Runtime>()
        .is_some_and(|r| r.shared.snapshot().island_enabled);
    if !enabled
        || event.kind == "deactivated"
        || (event.kind == "microphone" && event.text == "muted")
    {
        state.revision.fetch_add(1, SeqCst);
        state.active.store(false, SeqCst);
        let _ = window.hide();
        return;
    }
    if event.kind == "activated" || event.kind == "heard" {
        state.revision.fetch_add(1, SeqCst);
        state.active.store(true, SeqCst);
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let width = window
                .outer_size()
                .map(|s| s.width as i32)
                .unwrap_or((440.0 * scale) as i32);
            let _ = window.set_position(PhysicalPosition::new(
                area.position.x + (area.size.width as i32 - width) / 2,
                area.position.y + (18.0 * scale) as i32,
            ));
        }
        // focusable=false and click-through preserve the input field used by dictation.
        let _ = window.set_ignore_cursor_events(true);
        let _ = window.show();
        return;
    }
    if !state.active.load(SeqCst) {
        return;
    }
    let idle = event.kind == "status"
        && (event.text.starts_with("Ожидаю")
            || event.text == "Диктовка завершена"
            || event.text == "Микрофон отключён");
    if idle || matches!(event.kind.as_str(), "info" | "error") {
        let revision = state.revision.fetch_add(1, SeqCst) + 1;
        let counter = state.revision.clone();
        let app = app.clone();
        let delay = if event.kind == "error" { 6000 } else { 3000 };
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            if counter.load(SeqCst) == revision {
                app.state::<Island>().active.store(false, SeqCst);
                if let Some(window) = app.get_webview_window("island") {
                    let _ = window.hide();
                }
            }
        });
    } else if event.kind == "status" && !event.text.starts_with("Ожидаю") {
        state.revision.fetch_add(1, SeqCst);
    }
}
