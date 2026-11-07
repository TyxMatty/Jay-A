//! Jay-A's window and tray icon, and the commands over the audio engine (crates/jay-audio).

// Prevents an additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod soundboard;
#[cfg(windows)]
mod convert;
#[cfg(windows)]
mod editor;
#[cfg(windows)]
mod cable_setup;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};

/// The tray menu's items: built in English, relabelled by the page in its language.
struct TrayItems {
    open: MenuItem<Wry>,
    stop: MenuItem<Wry>,
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
fn set_tray_labels(open: String, stop: String, quit: String, items: State<'_, TrayItems>) -> Result<(), String> {
    items.open.set_text(open).map_err(|e| e.to_string())?;
    items.stop.set_text(stop).map_err(|e| e.to_string())?;
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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            set_tray_labels,
            #[cfg(windows)]
            audio::list_mics,
            #[cfg(windows)]
            audio::mic_options,
            #[cfg(windows)]
            audio::set_mic_options,
            #[cfg(windows)]
            audio::list_outputs,
            #[cfg(windows)]
            audio::cable,
            #[cfg(windows)]
            audio::set_cable,
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
            audio::preset_delete,
            #[cfg(windows)]
            soundboard::soundboard_state,
            #[cfg(windows)]
            soundboard::soundboard_import,
            #[cfg(windows)]
            soundboard::soundboard_import_path,
            #[cfg(windows)]
            convert::pick_audio_files,
            #[cfg(windows)]
            convert::pick_folder,
            #[cfg(windows)]
            convert::convert_start,
            #[cfg(windows)]
            convert::convert_cancel,
            #[cfg(windows)]
            cable_setup::cable_install,
            #[cfg(windows)]
            cable_setup::cable_site,
            #[cfg(windows)]
            editor::editor_open,
            #[cfg(windows)]
            editor::editor_view,
            #[cfg(windows)]
            editor::editor_edit,
            #[cfg(windows)]
            editor::editor_undo,
            #[cfg(windows)]
            editor::editor_play,
            #[cfg(windows)]
            editor::editor_stop,
            #[cfg(windows)]
            editor::editor_export,
            #[cfg(windows)]
            soundboard::soundboard_update,
            #[cfg(windows)]
            soundboard::soundboard_remove,
            #[cfg(windows)]
            soundboard::soundboard_settings,
            #[cfg(windows)]
            soundboard::soundboard_play,
            #[cfg(windows)]
            soundboard::soundboard_stop
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
            let stop = MenuItem::with_id(app, "stop", "Stop soundboard", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &stop, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("the app icon"))
                .tooltip("Jay-A")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    #[cfg(windows)]
                    "stop" => app.state::<soundboard::SoundboardState>().stop_all(&app.state::<audio::MicState>()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            app.manage(TrayItems { open, stop, quit });
            #[cfg(windows)]
            {
                let config_dir = app.path().app_config_dir()?;
                app.manage(audio::MicState::load(&config_dir));
                app.manage(audio::ChainState::load(&config_dir));
                audio::start_link(app.handle());
                app.state::<audio::MicState>().resume(&app.state::<audio::ChainState>());
                app.manage(soundboard::SoundboardState::load(config_dir, app.path().app_data_dir()?));
                soundboard::register_hotkeys(app.handle());
                app.manage(convert::ConvertState::default());
                app.manage(editor::EditorState::default());
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
