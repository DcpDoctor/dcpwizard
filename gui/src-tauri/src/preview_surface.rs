//! Hosts libmpv's OpenGL output in a `GtkGLArea` docked above the webview.
//!
//! Native Wayland cannot reparent another process's window, so the preview has
//! to render in-process. The GL area is packed into the box tauri already put
//! the webview in, as its sibling. Nothing may be inserted between the webview
//! and that box, or between the box and the window: tauri-runtime-wry's
//! undecorated-resizing hook walks exactly that chain on every mouse press and
//! panics if it does not end at the `GtkWindow`.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use gtk::glib;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use postkit::mpv_render::{MpvRenderPlayer, NativeDisplay};
use std::sync::Arc;

/// `GL_DRAW_FRAMEBUFFER_BINDING`. GtkGLArea renders into a framebuffer it owns
/// and offers no getter for it, so mpv's target has to be read back from GL.
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;

/// GtkGLArea composites its framebuffer top row first, the opposite of what
/// mpv draws by default, so the image arrives upside down without this.
const FLIP_Y: bool = true;

/// A closed preview keeps a one pixel transparent strip rather than hiding:
/// an unrealized GL area has no context, and advanced control needs the render
/// loop still answering mpv.
const CLOSED_PANE_HEIGHT: i32 = 1;

#[derive(Clone, Copy)]
struct PaneState {
    height: i32,
    visible: bool,
}

impl Default for PaneState {
    fn default() -> Self {
        Self {
            height: CLOSED_PANE_HEIGHT,
            visible: false,
        }
    }
}

enum SurfaceEvent {
    Redraw,
    Layout,
}

pub struct EmbeddedPreview {
    player: Arc<MpvRenderPlayer>,
    pane: Arc<Mutex<PaneState>>,
    events: async_channel::Sender<SurfaceEvent>,
}

impl EmbeddedPreview {
    pub fn player(&self) -> &MpvRenderPlayer {
        &self.player
    }

    /// Show or hide the video pane, `height` in the same logical pixels the
    /// page lays itself out in.
    pub fn set_pane(&self, visible: bool, height: i32) {
        *self.pane.lock().unwrap() = PaneState { height, visible };
        let _ = self.events.try_send(SurfaceEvent::Layout);
    }
}

/// Dock a GL area above the window's webview and hand back the player driving
/// it. Everything here touches GTK, so it must run on the main thread.
pub fn attach(window: &tauri::WebviewWindow) -> Result<EmbeddedPreview, String> {
    let webview_box = window.default_vbox().map_err(|e| e.to_string())?;

    let player = Arc::new(MpvRenderPlayer::new()?);
    let gl_area = gtk::GLArea::new();
    gl_area.set_has_depth_buffer(false);
    gl_area.set_has_stencil_buffer(false);
    let pane = Arc::new(Mutex::new(PaneState::default()));
    apply_pane(&gl_area, PaneState::default());

    let (events, incoming) = async_channel::unbounded::<SurfaceEvent>();

    gl_area.connect_realize({
        let player = Arc::clone(&player);
        let events = events.clone();
        move |area| bind_render_context(area, &player, &events)
    });

    gl_area.connect_render({
        let player = Arc::clone(&player);
        move |area, _context| {
            let scale = area.scale_factor();
            let width = area.allocated_width() * scale;
            let height = area.allocated_height() * scale;
            if let Err(error) = player.render_opengl(current_framebuffer(), width, height, FLIP_Y) {
                eprintln!("[preview] render failed: {error}");
            }
            player.report_swap();
            glib::Propagation::Stop
        }
    });

    // Signals first: packing into a box that is already on screen realizes the
    // area right away, and a realize handler connected after that never runs,
    // leaving every draw without a render context.
    webview_box.pack_start(&gl_area, false, true, 0);
    webview_box.reorder_child(&gl_area, 0);
    gl_area.show();
    if gl_area.is_realized() {
        bind_render_context(&gl_area, &player, &events);
    }

    spawn_event_pump(incoming, gl_area, Arc::clone(&player), Arc::clone(&pane));

    Ok(EmbeddedPreview {
        player,
        pane,
        events,
    })
}

/// Hand mpv the GL area's context. Reached from the realize signal and, when
/// the window is already on screen, directly from `attach`, so it has to
/// tolerate being called twice.
fn bind_render_context(
    gl_area: &gtk::GLArea,
    player: &Arc<MpvRenderPlayer>,
    events: &async_channel::Sender<SurfaceEvent>,
) {
    if player.is_initialized() {
        return;
    }
    gl_area.make_current();
    if let Some(error) = gl_area.error() {
        eprintln!("[preview] GL area failed to realize: {error}");
        return;
    }
    let native_display = native_display();
    if native_display.is_none() {
        eprintln!("[preview] no native display handle, hardware decode will be off");
    }
    if let Err(error) = player.init_opengl(resolve_gl_symbol, ptr::null_mut(), native_display) {
        eprintln!("[preview] libmpv OpenGL init failed: {error}");
        return;
    }
    let events = events.clone();
    player.set_update_callback(move || {
        let _ = events.try_send(SurfaceEvent::Redraw);
    });
    eprintln!(
        "[preview] GL renderer: {} ({})",
        gl_string(GL_RENDERER),
        gl_string(GL_VERSION)
    );
}

fn apply_pane(gl_area: &gtk::GLArea, pane: PaneState) {
    let active = pane.visible && pane.height > 0;
    gl_area.set_size_request(
        -1,
        if active {
            pane.height
        } else {
            CLOSED_PANE_HEIGHT
        },
    );
    gl_area.set_opacity(if active { 1.0 } else { 0.0 });
}

/// The main-thread half of the render loop. Advanced control makes calling
/// `wants_redraw` after every update callback mandatory, so it happens here
/// rather than being folded into the draw handler, which GTK may skip.
fn spawn_event_pump(
    incoming: async_channel::Receiver<SurfaceEvent>,
    gl_area: gtk::GLArea,
    player: Arc<MpvRenderPlayer>,
    pane: Arc<Mutex<PaneState>>,
) {
    glib::MainContext::default().spawn_local(async move {
        while let Ok(event) = incoming.recv().await {
            match event {
                SurfaceEvent::Redraw => {
                    if !gl_area.is_realized() {
                        continue;
                    }
                    gl_area.make_current();
                    if player.wants_redraw() {
                        gl_area.queue_render();
                    }
                }
                SurfaceEvent::Layout => {
                    let current = *pane.lock().unwrap();
                    apply_pane(&gl_area, current);
                }
            }
        }
    });
}

fn current_framebuffer() -> i32 {
    let mut framebuffer: i32 = 0;
    let Some(get_integerv) = gl_get_integerv() else {
        return 0;
    };
    unsafe { get_integerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut framebuffer) };
    framebuffer
}

type GlGetIntegerv = unsafe extern "C" fn(name: u32, values: *mut i32);
type GlGetString = unsafe extern "C" fn(name: u32) -> *const c_char;

fn gl_get_integerv() -> Option<GlGetIntegerv> {
    static ENTRY_POINT: OnceLock<usize> = OnceLock::new();
    let address = *ENTRY_POINT.get_or_init(|| gl_symbol("glGetIntegerv") as usize);
    (address != 0).then(|| unsafe { std::mem::transmute::<usize, GlGetIntegerv>(address) })
}

fn gl_string(name: u32) -> String {
    let address = gl_symbol("glGetString") as usize;
    if address == 0 {
        return "unknown".to_string();
    }
    let get_string = unsafe { std::mem::transmute::<usize, GlGetString>(address) };
    let value = unsafe { get_string(name) };
    if value.is_null() {
        return "unknown".to_string();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

/// The `wl_display` or X11 `Display` behind GDK's display, which mpv needs to
/// open a VA display for hardware decoding. gtk-rs binds neither accessor, so
/// they are looked up in the GDK already loaded into this process, guarded by
/// the display's own GType name.
fn native_display() -> Option<NativeDisplay> {
    let display = gtk::gdk::Display::default()?;
    let handle: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
    let (accessor, wrap): (&str, fn(*mut c_void) -> NativeDisplay) = match display.type_().name() {
        "GdkWaylandDisplay" => ("gdk_wayland_display_get_wl_display", NativeDisplay::Wayland),
        "GdkX11Display" => ("gdk_x11_display_get_xdisplay", NativeDisplay::X11),
        _ => return None,
    };
    let address = library_symbol(c"libgdk-3.so.0", accessor)? as usize;
    let get_native: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
        unsafe { std::mem::transmute(address) };
    let native = unsafe { get_native(handle as *mut c_void) };
    (!native.is_null()).then(|| wrap(native))
}

unsafe extern "C" fn resolve_gl_symbol(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        return ptr::null_mut();
    };
    gl_symbol(name)
}

/// Resolve a GL, EGL or GLX entry point through libepoxy, the dispatcher GTK
/// itself renders through, so mpv and GTK agree on which driver is in use.
/// libepoxy exports every entry point as a pointer variable named `epoxy_<name>`
/// holding a stub that resolves on first call, so the symbol address is the
/// address of that variable rather than of the function.
fn gl_symbol(name: &str) -> *mut c_void {
    let Some(slot) = library_symbol(c"libepoxy.so.0", &format!("epoxy_{name}")) else {
        return ptr::null_mut();
    };
    unsafe { *(slot as *const *mut c_void) }
}

/// These libraries are already loaded by GTK, so dlopen only bumps a refcount.
fn library_symbol(library_name: &CStr, symbol: &str) -> Option<*mut c_void> {
    let library =
        unsafe { libc::dlopen(library_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if library.is_null() {
        return None;
    }
    let symbol = CString::new(symbol).ok()?;
    let address = unsafe { libc::dlsym(library, symbol.as_ptr()) };
    (!address.is_null()).then_some(address)
}
