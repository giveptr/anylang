#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![warn(unused_qualifications)]

mod backup;
mod cancel;
mod canvas;
mod commands;
mod engine;
mod events;
mod gate;
mod hash;
mod job;
mod llm;
mod picks;
mod progress;
mod project;
mod scope;
mod service;
mod session;
mod settings;
mod store;
mod trace;
mod tuning;
mod walk;

use std::sync::Arc;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events};

fn build_specta() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::pick_game,
            commands::font_shown,
            commands::pictures,
            commands::picture_shown,
            commands::save_picture,
            commands::copy_picture,
            commands::keep_picture,
            commands::paste_picture,
            commands::replacement_shown,
            commands::picture_kinds,
            commands::pictures_at_once,
            commands::save_project,
            commands::preview_prompt,
            commands::survey,
            commands::prepare_game,
            commands::translate_scope,
            commands::translate_line,
            commands::live,
            commands::stop_scope,
            commands::stop_line,
            commands::export_scope,
            commands::forget_logs,
            commands::clear_scope,
            commands::revert_scope,
            commands::close_project,
            commands::delete_project,
            commands::list_rows,
            commands::search,
            commands::exclude_scope,
            commands::read_lines,
            commands::read_lines_around,
            commands::save_entry,
            commands::load_settings,
            commands::save_settings,
        ])
        .events(collect_events![
            events::FileStarted,
            events::FileDone,
            events::BatchDone,
            events::Notice,
            events::RunState,
            events::Preparing,
        ])
        .typ::<project::Mood>()
}

#[tokio::main]
async fn main() {
    trace::listening();
    tauri::async_runtime::set(tokio::runtime::Handle::current());

    let builder = build_specta();

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/lib/bindings.ts",
        )
        .expect("failed to export the TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            app.manage(cancel::Runs::default());
            app.manage(cancel::Seeker::default());
            app.manage(cancel::Solo::default());
            app.manage(Arc::new(service::lines::Sheets::default()));
            app.manage(gate::Gate::default());
            app.manage(session::Session::default());

            let window = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .expect("a window in the config");
            tauri::WebviewWindowBuilder::from_config(app.handle(), &window)?
                .data_directory(store::webview_dir()?)
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
