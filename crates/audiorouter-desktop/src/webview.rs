use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
#[cfg(windows)]
use tao::platform::windows::WindowExtWindows;
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;

use crate::settings::DesktopSettings;
use crate::AudioRouterApp;

const RESIZE_SAVE_DELAY: Duration = Duration::from_millis(500);
const APP_ICON_RGBA: &[u8] = include_bytes!("../../../assets/icons/audiorouter-icon-64.rgba");

fn save_window_size(state: &Arc<Mutex<AudioRouterApp>>, width: u32, height: u32) {
    if let Ok(mut app) = state.lock() {
        app.settings.window_width = width;
        app.settings.window_height = height;
        let _ = app.settings.save();
    }
}

pub fn run() -> wry::Result<()> {
    let event_loop = EventLoop::new();
    let saved_settings = DesktopSettings::load();
    let webview_data_directory = DesktopSettings::data_directory().ok();
    if let Some(directory) = webview_data_directory.as_ref() {
        let _ = std::fs::create_dir_all(directory);
    }
    let app_icon = Icon::from_rgba(APP_ICON_RGBA.to_vec(), 64, 64).ok();
    let window = WindowBuilder::new()
        .with_title("AudioRouter")
        .with_window_icon(app_icon)
        .with_inner_size(LogicalSize::new(
            saved_settings.window_width as f64,
            saved_settings.window_height as f64,
        ))
        .build(&event_loop)
        .expect("创建 AudioRouter 窗口失败");
    #[cfg(windows)]
    let mut tray = crate::tray::TrayController::new(window.hwnd() as _);
    let state = Arc::new(Mutex::new(AudioRouterApp::default()));
    let ipc_state = Arc::clone(&state);
    let html = include_str!("../../../assets/ui/pc-ui-mock.html");
    let app_info = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDirectory": webview_data_directory.as_ref().map(|path| path.display().to_string()),
        "iconSvg": include_str!("../../../assets/icons/audiorouter-mark.svg"),
    });
    let mut web_context = wry::WebContext::new(webview_data_directory);
    let webview = WebViewBuilder::with_web_context(&mut web_context)
        .with_initialization_script(&format!("window.__audioRouterAppInfo = {app_info};"))
        .with_html(html)
        .with_ipc_handler(move |request| {
            if let Ok(command) = serde_json::from_str::<serde_json::Value>(request.body()) {
                if let Ok(mut app) = ipc_state.lock() {
                    app.handle_web_command(&command);
                }
            }
        })
        .build(&window)?;

    let mut last_tick = Instant::now() - Duration::from_millis(100);
    let mut pending_resize: Option<(u32, u32, Instant)> = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(16));
        #[cfg(windows)]
        if let Some(tray) = tray.as_mut() {
            if tray.take_quit_request() {
                if let Some((width, height, _)) = pending_resize.take() {
                    save_window_size(&state, width, height);
                }
                tray.set_visible(false);
                *control_flow = ControlFlow::Exit;
                return;
            }
            if tray.take_restore_request() {
                tray.set_visible(false);
                window.set_visible(true);
            }
        }
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let logical_size = size.to_logical::<f64>(window.scale_factor());
                pending_resize = Some((
                    logical_size.width.round() as u32,
                    logical_size.height.round() as u32,
                    Instant::now(),
                ));
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                let width = size.width.round() as u32;
                let height = size.height.round() as u32;
                pending_resize = None;
                save_window_size(&state, width, height);
                #[cfg(windows)]
                {
                    let minimize_to_tray = state
                        .lock()
                        .is_ok_and(|app| app.settings.exit_behavior == "minimize_to_tray");
                    if minimize_to_tray && tray.as_mut().is_some_and(|tray| tray.set_visible(true))
                    {
                        window.set_visible(false);
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                #[cfg(not(windows))]
                {
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ if last_tick.elapsed() >= Duration::from_millis(16) => {
                last_tick = Instant::now();
                if pending_resize.is_some_and(|(_, _, at)| at.elapsed() >= RESIZE_SAVE_DELAY) {
                    if let Some((width, height, _)) = pending_resize.take() {
                        save_window_size(&state, width, height);
                    }
                }
                if let Ok(mut app) = state.lock() {
                    app.tick_backend();
                    let payload = app.web_state_json().to_string();
                    let script = format!("window.__audioRouterApplyState({payload});");
                    let _ = webview.evaluate_script(&script);
                    if let Some(message) = app.transport_error.take() {
                        let message = serde_json::to_string(&message)
                            .expect("connection error should serialize");
                        let script = format!("window.__audioRouterShowTransportError({message});");
                        let _ = webview.evaluate_script(&script);
                    }
                }
            }
            _ => {}
        }
    });
}
