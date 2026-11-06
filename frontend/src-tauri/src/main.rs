//! Jay-A's window and tray icon, and the commands over the audio engine (crates/jay-audio).

// Prevents an additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod audio;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};

/// The tray menu's items: built in English, relabelled by the page in its language.
struct TrayItems {
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// `--background`: tray only, no window. Jay-C starts Jay-A this way (docs/ARQUITETURA.md, section 6).
fn background(args: &[String]) -> bool {
    args.iter().any(|a| a == "--background")
}

#[tauri::command]
fn set_tray_labels(open: String, quit: String, items: State<'_, TrayItems>) -> Result<(), String> {
    items.open.set_text(open).map_err(|e| e.to_string())?;
    items.quit.set_text(quit).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        // First plugin: a second launch exits at once; it shows the running Jay-A unless it's Jay-C's
        // --background start, which mustn't pop the window up in the middle of a call.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !background(&args) {
                show_main(app);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            set_tray_labels,
            #[cfg(windows)]
            audio::list_mics,
            #[cfg(windows)]
            audio::mic_options,
            #[cfg(windows)]
            audio::set_mic_options,
            #[cfg(windows)]
            audio::start_mic,
            #[cfg(windows)]
            audio::stop_mic,
            #[cfg(windows)]
            audio::set_monitor,
            #[cfg(windows)]
            audio::link_state,
            #[cfg(windows)]
            audio::effects_catalog,
            #[cfg(windows)]
            audio::chain_state,
            #[cfg(windows)]
            audio::chain_edit,
            #[cfg(windows)]
            audio::chain_set_param,
            #[cfg(windows)]
            audio::chain_load,
            #[cfg(windows)]
            audio::chain_export,
            #[cfg(windows)]
            audio::presets_list,
            #[cfg(windows)]
            audio::preset_save,
            #[cfg(windows)]
            audio::preset_delete
        ])
        .on_window_event(|window, event| {
            // Closing the window keeps Jay-A in the tray, so the filters keep working during calls.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                // But a microphone test ends: nobody is looking at the meter anymore. (Jay-C keeps
                // the mic if it's using it.)
                #[cfg(windows)]
                if window.state::<audio::MicState>().stop_test(&window.state::<audio::ChainState>()) {
                    let _ = window.emit("mic-stopped", ());
                }
            }
        })
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open Jay-A", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("the app icon"))
                .tooltip("Jay-A")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            app.manage(TrayItems { open, quit });
            #[cfg(windows)]
            {
                let config_dir = app.path().app_config_dir()?;
                app.manage(audio::MicState::load(&config_dir));
                app.manage(audio::ChainState::load(&config_dir));
                audio::start_link(app.handle());
            }
            // The window starts hidden (tauri.conf.json), so --background never flashes it.
            if !background(&std::env::args().collect::<Vec<_>>()) {
                show_main(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Jay-A");
}
