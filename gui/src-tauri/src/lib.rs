#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// pipeline + preview

use tauri::Manager;

#[cfg(target_os = "linux")]
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

mod pipeline;
mod preview_server;
#[cfg(all(target_os = "linux", feature = "embedded-preview"))]
mod preview_surface;
mod timeline;

#[cfg(unix)]
fn fork_terminal_guard() {
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) == 0 {
            return;
        }

        let mut saved: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut saved);

        let pid = libc::fork();
        if pid < 0 {
            return;
        }
        if pid > 0 {
            let mut status: libc::c_int = 0;
            libc::waitpid(pid, &mut status, 0);
            libc::usleep(100_000);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &saved);
            libc::system(c"stty sane 2>/dev/null".as_ptr());
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };
            std::process::exit(exit_code);
        }
        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        if devnull >= 0 {
            libc::dup2(devnull, libc::STDIN_FILENO);
            libc::close(devnull);
        }
    }
}

/// Draw the preview inside the app window when libmpv is linked in, and fall
/// back to a separate mpv window otherwise.
fn create_preview_player(app: &tauri::App) -> preview_server::PreviewPlayer {
    #[cfg(all(target_os = "linux", feature = "embedded-preview"))]
    if let Some(window) = app.get_window(MAIN_WINDOW_LABEL) {
        match preview_surface::attach(&window) {
            Ok(preview) => return preview_server::PreviewPlayer::Embedded(preview),
            Err(error) => eprintln!("[preview] embedded playback unavailable: {error}"),
        }
    }
    let _ = app;
    preview_server::new_player()
}

#[cfg(target_os = "linux")]
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let window = tauri::window::WindowBuilder::new(app, MAIN_WINDOW_LABEL)
        .title(MAIN_WINDOW_TITLE)
        .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
        .min_inner_size(MAIN_WINDOW_MINIMUM_WIDTH, MAIN_WINDOW_MINIMUM_HEIGHT)
        .build()?;
    let size = window.inner_size()?;
    let webview = tauri::webview::WebviewBuilder::new(
        MAIN_WEBVIEW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    );
    window.add_child(webview, tauri::LogicalPosition::new(0, 0), size)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    fork_terminal_guard();

    let job_queue = pipeline::JobQueue::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .manage(job_queue)
        .invoke_handler(tauri::generate_handler![
            preview_server::preview_load,
            preview_server::preview_play_pause,
            preview_server::preview_seek,
            preview_server::preview_seek_absolute,
            preview_server::preview_stop,
            preview_server::preview_load_dcp,
            preview_server::preview_get_position,
            preview_server::preview_get_duration,
            preview_server::preview_get_metadata,
            preview_server::preview_set_parent_wid,
            preview_server::preview_set_surface,
            preview_server::preview_is_embedded,
            pipeline::submit_job,
            pipeline::cancel_job,
            pipeline::pause_job,
            pipeline::resume_job,
            pipeline::list_jobs,
            pipeline::list_profiles,
            pipeline::create_vf,
            timeline::list_cpls,
            timeline::get_timeline,
        ])
        .setup(|app| {
            #[cfg(target_os = "linux")]
            create_main_window(app)?;
            app.manage(create_preview_player(app));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(player) = window.try_state::<preview_server::PreviewPlayer>() {
                    player.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
