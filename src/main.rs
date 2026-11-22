// No console window behind the tray icon on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon;
mod providers;

use providers::{Config, Provider};
use serde::Serialize;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent};

const TRAY_ID: &str = "copia";

#[derive(Serialize, Clone)]
struct Snapshot {
    providers: Vec<Provider>,
    /// Epoch seconds of the last finished refresh.
    updated_at: Option<u64>,
    loading: bool,
    demo: bool,
    /// Fractions left below which a bar turns amber / red (see icon.rs).
    warn_below: f64,
    low_below: f64,
}

impl Snapshot {
    fn new(providers: Vec<Provider>, updated_at: Option<u64>, loading: bool, demo: bool) -> Self {
        Snapshot {
            providers,
            updated_at,
            loading,
            demo,
            warn_below: icon::WARN_BELOW,
            low_below: icon::LOW_BELOW,
        }
    }
}

/// Where the tray icon is, in physical pixels, so the popover can sit by it.
#[derive(Clone, Copy)]
struct Anchor {
    centre_x: f64,
    top: f64,
    bottom: f64,
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    refresh: Mutex<mpsc::Sender<()>>,
    anchor: Mutex<Option<Anchor>>,
    hidden_at: Mutex<Instant>,
}

#[tauri::command]
fn get_snapshot(shared: State<Shared>) -> Snapshot {
    shared.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn refresh(shared: State<Shared>) {
    let _ = shared.refresh.lock().unwrap().send(());
}

/// Escape in the popover.
#[tauri::command]
fn hide(window: WebviewWindow) {
    let _ = window.hide();
}

/// The page tells us how big its content is; size the popover to match.
#[tauri::command]
fn fit(window: WebviewWindow, shared: State<Shared>, width: f64, height: f64) {
    let size = tauri::LogicalSize::new(width.clamp(180.0, 900.0), height.clamp(120.0, 600.0));
    let _ = window.set_size(size);
    place(&window, *shared.anchor.lock().unwrap());
}

fn place(window: &WebviewWindow, anchor: Option<Anchor>) {
    let Ok(size) = window.outer_size() else { return };
    let (width, height) = (size.width as f64, size.height as f64);
    let monitor = anchor
        .and_then(|a| window.monitor_from_point(a.centre_x, a.top).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let (left, top) = (monitor.position().x as f64, monitor.position().y as f64);
    let (right, bottom) = (left + monitor.size().width as f64, top + monitor.size().height as f64);
    let margin = 8.0 * monitor.scale_factor();

    let (x, y) = match anchor {
        // Tray at the top of the screen (macOS, some Linux): hang below it.
        Some(a) if a.top < top + (bottom - top) / 2.0 => (a.centre_x - width / 2.0, a.bottom + margin / 2.0),
        // Tray at the bottom (Windows): sit above it.
        Some(a) => (a.centre_x - width / 2.0, a.top - height - margin / 2.0),
        // No click position (Linux menus): top-right corner.
        None => (right - width - margin, top + margin * 4.0),
    };
    let x = x.clamp(left + margin, (right - width - margin).max(left + margin));
    let y = y.clamp(top, (bottom - height - margin).max(top));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn show(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    place(&window, *app.state::<Shared>().anchor.lock().unwrap());
    let _ = window.show();
    let _ = window.set_focus();
}

fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    // Clicking the tray while the popover is open first blurs (and hides) it;
    // don't let that same click reopen it.
    if app.state::<Shared>().hidden_at.lock().unwrap().elapsed() < Duration::from_millis(250) {
        return;
    }
    show(app);
}

fn update_tray(app: &AppHandle, providers: &[Provider]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let shown: Vec<&Provider> = providers.iter().filter(|p| p.configured).collect();

    // One bar per provider: what is left of its tightest quota.
    let levels: Vec<Option<f64>> = shown
        .iter()
        .map(|p| p.quotas.iter().map(|q| 1.0 - q.used).reduce(f64::min))
        .collect();
    let template = cfg!(target_os = "macos");
    let (rgba, width, height) = icon::render(&levels, template);
    let _ = tray.set_icon(Some(Image::new_owned(rgba, width, height)));
    let _ = tray.set_icon_as_template(template);

    let lines: Vec<String> = shown
        .iter()
        .map(|p| match (&p.error, p.quotas.is_empty()) {
            (Some(error), _) => format!("{}: {error}", p.name),
            (None, true) => format!("{}: no data", p.name),
            (None, false) => {
                let parts: Vec<String> = p
                    .quotas
                    .iter()
                    .map(|q| format!("{} {:.0}%", q.label, (1.0 - q.used) * 100.0))
                    .collect();
                format!("{}: {} left", p.name, parts.join(", "))
            }
        })
        .collect();
    let tooltip = if lines.is_empty() {
        "Copia — no providers found".to_string()
    } else {
        lines.join("\n")
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

fn spawn_worker(app: AppHandle, wake: mpsc::Receiver<()>, demo: bool) {
    std::thread::spawn(move || loop {
        let shared = app.state::<Shared>();
        shared.snapshot.lock().unwrap().loading = true;
        let _ = app.emit("snapshot", ());

        // Re-read the config every round so edits apply without a restart.
        let config = Config::load();
        let providers = if demo { providers::demo() } else { providers::fetch_all(&config) };
        update_tray(&app, &providers);
        *shared.snapshot.lock().unwrap() =
            Snapshot::new(providers, Some(providers::now_secs()), false, demo);
        let _ = app.emit("snapshot", ());

        let minutes = config.refresh_minutes.unwrap_or(5).clamp(1, 240);
        let _ = wake.recv_timeout(Duration::from_secs(minutes * 60));
        while wake.try_recv().is_ok() {}
    });
}

fn main() {
    let demo = std::env::args().any(|a| a == "--demo") || std::env::var_os("COPIA_DEMO").is_some();
    // `copia --print`: fetch once, print the quotas as JSON, and exit. No tray.
    if std::env::args().any(|a| a == "--print") {
        let providers = if demo { providers::demo() } else { providers::fetch_all(&Config::load()) };
        println!("{}", serde_json::to_string_pretty(&providers).unwrap_or_default());
        return;
    }
    // Development aid: keep the popover open when it loses focus.
    let pinned = std::env::var_os("COPIA_PIN").is_some();
    let (refresh_tx, refresh_rx) = mpsc::channel();

    tauri::Builder::default()
        .manage(Shared {
            snapshot: Mutex::new(Snapshot::new(Vec::new(), None, true, demo)),
            refresh: Mutex::new(refresh_tx),
            anchor: Mutex::new(None),
            hidden_at: Mutex::new(Instant::now() - Duration::from_secs(1)),
        })
        .invoke_handler(tauri::generate_handler![get_snapshot, refresh, fit, hide])
        .on_window_event(move |window, event| match event {
            WindowEvent::Focused(false) if !pinned => {
                *window.state::<Shared>().hidden_at.lock().unwrap() = Instant::now();
                let _ = window.hide();
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .setup(move |app| {
            // A tray app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "show", "Show quotas", true, None::<&str>)?,
                    &MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "Quit Copia", true, None::<&str>)?,
                ],
            )?;
            let template = cfg!(target_os = "macos");
            let (rgba, width, height) = icon::render(&[], template);
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::new_owned(rgba, width, height))
                .icon_as_template(template)
                .tooltip("Copia")
                .menu(&menu)
                // Left click opens the bars; the menu is on right click.
                // (Linux trays only offer the menu, hence "Show quotas".)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show(app),
                    "refresh" => {
                        let _ = app.state::<Shared>().refresh.lock().unwrap().send(());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        let position = rect.position.to_physical::<f64>(1.0);
                        let size = rect.size.to_physical::<f64>(1.0);
                        *app.state::<Shared>().anchor.lock().unwrap() = Some(Anchor {
                            centre_x: position.x + size.width / 2.0,
                            top: position.y,
                            bottom: position.y + size.height,
                        });
                        toggle(app);
                    }
                })
                .build(app)?;

            spawn_worker(app.handle().clone(), refresh_rx, demo);
            if pinned {
                show(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Copia");
}
