pub use postkit::mpv::MpvPlayer;

const APP_NAME: &str = "DCPWizard";

/// Preview playback backend. The embedded one draws into a GL surface this app
/// owns; the spawned one runs mpv as a separate window, which is all that is
/// possible without libmpv at build time.
pub enum PreviewPlayer {
    Spawned(MpvPlayer),
    #[cfg(feature = "embedded-preview")]
    Embedded(crate::preview_surface::EmbeddedPreview),
}

pub fn new_player() -> PreviewPlayer {
    PreviewPlayer::Spawned(MpvPlayer::new(APP_NAME))
}

/// Route every call to whichever backend is in use. The two players expose the
/// same operations, so each arm is the same call on a different receiver.
macro_rules! dispatch {
    ($self:expr, $call:ident ( $( $argument:expr ),* )) => {
        match $self {
            PreviewPlayer::Spawned(player) => player.$call($( $argument ),*),
            #[cfg(feature = "embedded-preview")]
            PreviewPlayer::Embedded(preview) => preview.player().$call($( $argument ),*),
        }
    };
}

impl PreviewPlayer {
    pub fn load_file(&self, path: &str) -> Result<(), String> {
        dispatch!(self, load_file(path))
    }

    pub fn load_package_dir(&self, dir_path: &str) -> Result<(), String> {
        dispatch!(self, load_package_dir(dir_path))
    }

    pub fn play_pause(&self) -> Result<(), String> {
        dispatch!(self, play_pause())
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        dispatch!(self, seek(seconds))
    }

    pub fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        dispatch!(self, seek_absolute(seconds))
    }

    pub fn stop(&self) -> Result<(), String> {
        dispatch!(self, stop())
    }

    pub fn get_position(&self) -> Result<f64, String> {
        dispatch!(self, get_position())
    }

    pub fn get_duration(&self) -> Result<f64, String> {
        dispatch!(self, get_duration())
    }

    pub fn get_metadata(&self) -> Result<String, String> {
        dispatch!(self, get_metadata())
    }

    /// Shut the spawned mpv process down. The embedded player releases its mpv
    /// core when the app state is dropped.
    pub fn kill(&self) {
        match self {
            PreviewPlayer::Spawned(player) => player.kill(),
            #[cfg(feature = "embedded-preview")]
            PreviewPlayer::Embedded(_) => {}
        }
    }
}

#[tauri::command]
pub fn preview_set_parent_wid(
    wid: u64,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    match &*state {
        PreviewPlayer::Spawned(player) => {
            player.set_parent_wid(wid);
            if player.is_alive() {
                player.kill();
                player.start_mpv()?;
            }
            Ok(())
        }
        #[cfg(feature = "embedded-preview")]
        PreviewPlayer::Embedded(_) => Ok(()),
    }
}

/// Report where the page's video placeholder sits, in CSS pixels from the
/// top-left of the webview, so the embedded surface can be moved to match.
#[tauri::command]
pub fn preview_set_surface(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    #[cfg(feature = "embedded-preview")]
    if let PreviewPlayer::Embedded(preview) = &*state {
        preview.set_surface(x, y, width, height, visible);
    }
    #[cfg(not(feature = "embedded-preview"))]
    let _ = (x, y, width, height, visible, state);
    Ok(())
}

/// True when video draws inside the app window rather than a separate one.
#[tauri::command]
pub fn preview_is_embedded(state: tauri::State<'_, PreviewPlayer>) -> bool {
    #[cfg(feature = "embedded-preview")]
    {
        matches!(&*state, PreviewPlayer::Embedded(_))
    }
    #[cfg(not(feature = "embedded-preview"))]
    {
        let _ = state;
        false
    }
}

#[tauri::command]
pub fn preview_load(
    file_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.load_file(&file_path)
}

#[tauri::command]
pub fn preview_play_pause(state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.play_pause()
}

#[tauri::command]
pub fn preview_seek(seconds: f64, state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.seek(seconds)
}

#[tauri::command]
pub fn preview_seek_absolute(
    seconds: f64,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.seek_absolute(seconds)
}

#[tauri::command]
pub fn preview_stop(state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.stop()
}

#[tauri::command]
pub fn preview_get_position(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.get_position()
}

#[tauri::command]
pub fn preview_get_duration(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.get_duration()
}

#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, PreviewPlayer>) -> Result<String, String> {
    state.get_metadata()
}

#[tauri::command]
pub fn preview_load_dcp(
    dir_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.load_package_dir(&dir_path)
}
