//! Hosts libmpv's OpenGL output in a `GtkGLArea` layered over the webview.
//!
//! Native Wayland cannot reparent another process's window, so the preview has
//! to render in-process. The GL area sits in a `GtkOverlay` above tauri's
//! webview box and is positioned from the page: the frontend reports where its
//! placeholder element sits and the area is moved to match.

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

#[derive(Clone, Copy, Default)]
struct SurfaceRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
}

enum SurfaceEvent {
    Redraw,
    Layout,
}

pub struct EmbeddedPreview {
    player: Arc<MpvRenderPlayer>,
    rect: Arc<Mutex<SurfaceRect>>,
    events: async_channel::Sender<SurfaceEvent>,
}

impl EmbeddedPreview {
    pub fn player(&self) -> &MpvRenderPlayer {
        &self.player
    }

    /// Move the video surface to where the page says its placeholder is, in CSS
    /// pixels from the top-left of the webview.
    pub fn set_surface(&self, x: i32, y: i32, width: i32, height: i32, visible: bool) {
        *self.rect.lock().unwrap() = SurfaceRect {
            x,
            y,
            width,
            height,
            visible,
        };
        let _ = self.events.try_send(SurfaceEvent::Layout);
    }
}

/// Put a GL area over the window's webview and hand back the player driving it.
/// Everything here touches GTK, so it must run on the main thread.
pub fn attach(window: &tauri::Window) -> Result<EmbeddedPreview, String> {
    let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
    let webview_box = window.default_vbox().map_err(|e| e.to_string())?;

    let player = Arc::new(MpvRenderPlayer::new()?);
    let gl_area = gtk::GLArea::new();
    gl_area.set_has_depth_buffer(false);
    gl_area.set_has_stencil_buffer(false);
    gl_area.set_halign(gtk::Align::Start);
    gl_area.set_valign(gtk::Align::Start);
    // The area stays realized even with no preview on screen: mpv's render
    // context lives on its GL context, and advanced control needs the render
    // loop answering. An inactive preview shrinks to a transparent pixel.
    let rect = Arc::new(Mutex::new(SurfaceRect::default()));
    apply_rect(&gl_area, SurfaceRect::default());

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

    // Signals first: adding the area to a window that is already on screen
    // realizes it right away, and a realize handler connected after that never
    // runs, leaving every draw without a render context.
    let overlay = gtk::Overlay::new();
    gtk_window.remove(&webview_box);
    overlay.add(&webview_box);
    overlay.add_overlay(&gl_area);
    gtk_window.add(&overlay);
    overlay.show_all();
    if gl_area.is_realized() {
        bind_render_context(&gl_area, &player, &events);
    }

    spawn_event_pump(incoming, gl_area, Arc::clone(&player), Arc::clone(&rect));

    Ok(EmbeddedPreview {
        player,
        rect,
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

fn apply_rect(gl_area: &gtk::GLArea, rect: SurfaceRect) {
    let active = rect.visible && rect.width > 0 && rect.height > 0;
    gl_area.set_margin_start(if active { rect.x } else { 0 });
    gl_area.set_margin_top(if active { rect.y } else { 0 });
    gl_area.set_size_request(
        if active { rect.width } else { 1 },
        if active { rect.height } else { 1 },
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
    rect: Arc<Mutex<SurfaceRect>>,
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
                    let current = *rect.lock().unwrap();
                    apply_rect(&gl_area, current);
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
