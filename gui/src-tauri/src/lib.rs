#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// pipeline + preview

#[allow(unused_imports)]
use tauri::Manager;

mod pipeline;
#[cfg(unix)]
mod preview_server;
#[cfg(not(unix))]
mod preview_server_stub;
mod timeline;
#[cfg(not(unix))]
use preview_server_stub as preview_server;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    fork_terminal_guard();

    let mpv = preview_server::new_player();
    let job_queue = pipeline::JobQueue::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .manage(mpv)
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
        .setup(|_app| Ok(()))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(mpv) = window.try_state::<preview_server::MpvPlayer>() {
                    mpv.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
