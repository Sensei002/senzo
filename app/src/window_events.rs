use crate::{AppModel, AppModelField, MainWindow, NavigationController};
use core_env::DesktopEnv;
use slint::winit_030::{EventResult, winit};
use std::{
    cell::Cell,
    sync::Arc,
    time::{Duration, Instant},
};
use stremio_core::runtime::{
    Runtime, RuntimeAction,
    msg::{Action, ActionCtx},
};

const FOCUS_SYNC_DEBOUNCE: Duration = Duration::from_secs(2);

fn dispatch_focus_sync(runtime: &Runtime<DesktopEnv, AppModel>) {
    for action in [
        ActionCtx::PullAddonsFromAPI,
        ActionCtx::PullUserFromAPI { token: None },
        ActionCtx::SyncLibraryWithAPI,
        ActionCtx::PullNotifications,
    ] {
        runtime.dispatch(RuntimeAction {
            field: Some(AppModelField::Ctx),
            action: Action::Ctx(action),
        });
    }
}

fn is_torrent(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
}

pub fn install(
    ui: &MainWindow,
    runtime: Arc<Runtime<DesktopEnv, AppModel>>,
    navigation: NavigationController,
) {
    let last_focus_sync = Cell::new(Instant::now());
    let ui_weak = ui.as_weak();
    crate::window_hooks::register(ui, move |_window, event| {
        match event {
            winit::event::WindowEvent::Focused(true)
                if last_focus_sync.get().elapsed() >= FOCUS_SYNC_DEBOUNCE =>
            {
                last_focus_sync.set(Instant::now());
                dispatch_focus_sync(&runtime);
            }
            winit::event::WindowEvent::DroppedFile(path) if is_torrent(path) => {
                if let Some(ui) = ui_weak.upgrade() {
                    crate::deep_link::open_torrent_file(&ui, &runtime, &navigation, path.clone());
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

#[cfg(test)]
mod tests {
    use super::is_torrent;
    use std::path::Path;

    #[test]
    fn recognizes_torrent_extension_case_insensitively() {
        assert!(is_torrent(Path::new("movie.torrent")));
        assert!(is_torrent(Path::new("show.TORRENT")));
        assert!(!is_torrent(Path::new("video.mp4")));
    }
}
