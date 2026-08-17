#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

const MAIN_WINDOW_LABEL: &str = "main";
#[cfg(target_os = "linux")]
const MAIN_WEBVIEW_LABEL: &str = "main-webview";
#[cfg(target_os = "linux")]
const MAIN_WINDOW_TITLE: &str = "DCP Wizard — DCP Creator";
#[cfg(target_os = "linux")]
const MAIN_WINDOW_WIDTH: f64 = 900.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_HEIGHT: f64 = 700.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_MINIMUM_WIDTH: f64 = 700.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_MINIMUM_HEIGHT: f64 = 500.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_BACKGROUND: tauri::window::Color = tauri::window::Color(0, 0, 0, 255);

mod library;
mod pipeline;
mod timeline;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    guikit::startup::fork_terminal_guard();

    let job_queue = pipeline::JobQueue::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .manage(job_queue)
        .invoke_handler(tauri::generate_handler![
            guikit::preview::preview_load,
            guikit::preview::preview_play_pause,
            guikit::preview::preview_seek,
            guikit::preview::preview_seek_absolute,
            guikit::preview::preview_stop,
            guikit::preview::preview_load_dcp,
            guikit::preview::preview_get_position,
            guikit::preview::preview_get_duration,
            guikit::preview::preview_get_metadata,
            guikit::preview::preview_set_surface,
            guikit::preview::preview_is_embedded,
            guikit::preview::preview_set_overlays,
            guikit::preview::preview_set_decode_scale,
            guikit::preview::preview_set_subtitle_file,
            guikit::preview::preview_set_subtitle_visibility,
            pipeline::submit_job,
            pipeline::cancel_job,
            pipeline::pause_job,
            pipeline::resume_job,
            pipeline::list_jobs,
            pipeline::delete_dcp,
            pipeline::retitle_dcp,
            pipeline::disk_space,
            pipeline::list_profiles,
            pipeline::detect_source_crop,
            pipeline::subtitle_file_for_preview,
            pipeline::probe_audio_map,
            pipeline::isdcf_name_preview,
            pipeline::create_vf,
            library::library_list,
            library::library_add,
            library::library_remove,
            library::library_needs_duration,
            timeline::list_cpls,
            timeline::get_timeline,
        ])
        .setup(|app| {
            #[cfg(target_os = "linux")]
            guikit::startup::create_main_window(
                app,
                &guikit::startup::MainWindow {
                    label: MAIN_WINDOW_LABEL,
                    webview_label: MAIN_WEBVIEW_LABEL,
                    title: MAIN_WINDOW_TITLE,
                    width: MAIN_WINDOW_WIDTH,
                    height: MAIN_WINDOW_HEIGHT,
                    minimum_width: MAIN_WINDOW_MINIMUM_WIDTH,
                    minimum_height: MAIN_WINDOW_MINIMUM_HEIGHT,
                    background: MAIN_WINDOW_BACKGROUND,
                },
            )?;
            app.manage(guikit::preview::create_player(app, MAIN_WINDOW_LABEL));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
