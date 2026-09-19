//! # velqu-shell
//!
//! The native shell around a [`VelquView`]: window creation, the event loop,
//! DPI/resize handling, and presenting frames to the screen.
//!
//! M1 backend choice (see `docs/decisions/0001-m1-paint-backend.md`):
//! **winit + softbuffer**, i.e. a software blit to a native window. This keeps
//! M1 free of GPU dependencies and works on machines without accelerated
//! graphics; the rendering itself happens in `velqu-view` offscreen, so a
//! later GPU backend swap does not touch this crate's contract beyond how
//! frames reach the surface.
//!
//! Input is forwarded to the renderer's input gate (M4a/M4b, ADR
//! 0010/0011): pointer move/press/release, wheel scrolling, and Tab focus
//! cycling are handed to [`VelquView`], which answers from the cached
//! layout with zero additional layout passes. Interaction styling is real
//! now: when hover/active/focus state changes pixels the view says so and
//! the shell schedules a repaint (presentation-only, no relayout); the
//! CSS `cursor` maps onto the platform cursor. Escape still closes.
//! M4c1 adds keyboard editing: named keys become backend-independent
//! commands (arrows, Home/End, Backspace/Delete, Enter, Ctrl/Cmd+A) and
//! only non-command `KeyEvent.text` reaches the view as text insertion —
//! winit's `"\r"` for Enter or `"\t"` for Tab can never leak into a
//! control. M4c2 adds the clipboard: Ctrl/Cmd+C/X/V map to Copy/Cut/
//! Paste commands, and the shell installs an OS-backed clipboard provider
//! (arboard) when the session has one — the renderer itself only ever
//! sees the host `ClipboardProvider` interface, whose failures make cut
//! transactional. Routing is AltGr-safe (ADR 0013): Alt disqualifies
//! shortcut chords and text suppression, so Ctrl+Alt chords (e.g. `@` =
//! Ctrl+Alt+Q on German layouts) insert their produced text. The
//! clipboard adapter is torn down explicitly at loop exit. M4c3 adds
//! IME: winit's `Ime` events translate onto the view's backend-
//! independent composition API, and the shell owns enablement —
//! `set_ime_allowed` follows the focused control and
//! `set_ime_cursor_area` tracks the caret rect in physical pixels
//! through focus changes, caret/selection movement, scrolling, resizes,
//! and composition updates (ADR 0014). Window blur cancels an active
//! composition.

use std::fmt;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use velqu_view::{
    CursorStyle, FrameResult, KeyCommand, KeyModifiers, VelquError, VelquView, Viewport,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

/// Vertical px per wheel line notch (winit `LineDelta`); matches common
/// browser defaults and the visual scroll fixtures.
const WHEEL_LINE_PX: f32 = 40.0;

/// A fatal shell error (window, surface, or render failure).
#[derive(Debug)]
pub struct ShellError {
    /// Human-readable description; shell errors are not fine-grained in M1.
    pub message: String,
}

impl ShellError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ShellError {}

impl From<winit::error::EventLoopError> for ShellError {
    fn from(err: winit::error::EventLoopError) -> Self {
        ShellError::new(format!("event loop failed: {err}"))
    }
}

impl From<winit::error::OsError> for ShellError {
    fn from(err: winit::error::OsError) -> Self {
        ShellError::new(format!("window creation failed: {err}"))
    }
}

impl From<softbuffer::SoftBufferError> for ShellError {
    fn from(err: softbuffer::SoftBufferError) -> Self {
        ShellError::new(format!("surface failed: {err}"))
    }
}

impl From<VelquError> for ShellError {
    fn from(err: VelquError) -> Self {
        ShellError::new(format!("render failed: {err}"))
    }
}

impl From<velqu_view::InvalidViewport> for ShellError {
    fn from(err: velqu_view::InvalidViewport) -> Self {
        ShellError::new(format!("render failed: {err}"))
    }
}

/// Window configuration for [`run`].
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Window title.
    pub title: String,
    /// Initial window size in logical pixels.
    pub logical_size: (f32, f32),
    /// Whether the user may resize the window.
    pub resizable: bool,
    /// Automatically close the window after this duration (smoke tests).
    pub exit_after: Option<Duration>,
}

impl ShellConfig {
    /// Config with `title`, a 1024×640 logical window, resizable, no auto-exit.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            logical_size: (1024.0, 640.0),
            resizable: true,
            exit_after: None,
        }
    }

    /// Sets the initial logical window size.
    pub fn with_logical_size(mut self, width: f32, height: f32) -> Self {
        self.logical_size = (width, height);
        self
    }

    /// Enables or disables user resizing.
    pub fn with_resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Auto-closes the window after `duration`.
    pub fn with_exit_after(mut self, duration: Duration) -> Self {
        self.exit_after = Some(duration);
        self
    }
}

/// What the shell did before the window closed.
#[derive(Debug, Clone, Copy)]
pub struct ShellStats {
    /// Frames presented.
    pub frames: u64,
    /// Wall time the window was open.
    pub uptime: Duration,
}

/// A shell-side wakeup sent into the event loop by host threads (the
/// M6c watcher). The loop wakes from `Wait` and runs the host's idle
/// hook on the main thread — the view never crosses threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellUserEvent {
    /// Tracked sources may be stale: run the watch hook.
    Watch,
}

/// The host-side handle for waking the loop (re-exported so hosts
/// need no direct winit dependency).
pub type ShellProxy = winit::event_loop::EventLoopProxy<ShellUserEvent>;

/// The host hook invoked on the main thread after a
/// [`ShellUserEvent::Watch`]. Returns whether the document should
/// redraw (a watcher wakeup alone never forces rendering).
pub type WatchHook<'a> = Box<dyn FnMut(&mut VelquView) -> bool + 'a>;

/// Opens a native window and runs the event loop until the window closes.
///
/// The window shows live [`VelquView`] frames, re-rendering whenever the
/// window is resized or its DPI scale changes. Blocking; returns when the
/// user closes the window, `exit_after` elapses, or an error occurs.
pub fn run(config: ShellConfig, view: &mut VelquView) -> Result<ShellStats, ShellError> {
    run_inner(config, view, None)
}

/// [`run`] plus a host watch integration (M6c, ADR 0022):
/// `spawn_watcher` receives the loop proxy before the loop starts
/// (forward watcher wakeups with `send_event(ShellUserEvent::Watch)`),
/// and `hook` runs on the main thread per wake — drain notifications,
/// reconcile due reloads, and return whether to redraw.
pub fn run_with_watch(
    config: ShellConfig,
    view: &mut VelquView,
    spawn_watcher: impl FnOnce(winit::event_loop::EventLoopProxy<ShellUserEvent>) + 'static,
    hook: WatchHook<'_>,
) -> Result<ShellStats, ShellError> {
    run_inner(
        config,
        view,
        Some(WatchSetup {
            spawn: Box::new(spawn_watcher),
            hook,
        }),
    )
}

/// The watch integration handed to [`run_inner`]: the pre-loop
/// spawner (receives the proxy) and the idle hook.
struct WatchSetup<'a> {
    spawn: Box<dyn FnOnce(winit::event_loop::EventLoopProxy<ShellUserEvent>)>,
    hook: WatchHook<'a>,
}

fn run_inner(
    config: ShellConfig,
    view: &mut VelquView,
    watch: Option<WatchSetup<'_>>,
) -> Result<ShellStats, ShellError> {
    let event_loop = winit::event_loop::EventLoop::<ShellUserEvent>::with_user_event().build()?;
    let context = softbuffer::Context::new(event_loop.owned_display_handle())?;
    // OS clipboard (M4c2, ADR 0013): the shell is the host that owns
    // platform integration. When the session has no clipboard service
    // (headless CI), the view keeps its null provider — paste reads
    // nothing and cut refuses to destroy the selection (transactional).
    let clipboard: Option<std::rc::Rc<dyn velqu_view::ClipboardProvider>> =
        arboard::Clipboard::new().ok().map(|board| {
            let provider: std::rc::Rc<dyn velqu_view::ClipboardProvider> =
                std::rc::Rc::new(ArboardClipboard {
                    board: std::cell::RefCell::new(board),
                });
            view.set_clipboard_provider(provider.clone());
            provider
        });

    // `Wait` sleeps until input; a deadline (smoke tests) needs `WaitUntil`
    // so the loop wakes up to honor `exit_after` with no other events.
    // The watch proxy wakes the loop the same way.
    event_loop.set_control_flow(match config.exit_after {
        Some(exit_after) => winit::event_loop::ControlFlow::WaitUntil(Instant::now() + exit_after),
        None => winit::event_loop::ControlFlow::Wait,
    });

    // Split the watch setup: the hook moves into the app, the spawner
    // runs once with the loop proxy before the loop starts.
    let (spawn_watcher, watch_hook) = match watch {
        Some(WatchSetup { spawn, hook }) => (Some(spawn), Some(hook)),
        None => (None, None),
    };
    let mut app = ShellApp {
        config,
        view,
        context,
        window: None,
        surface: None,
        dirty: true,
        // NaN until the first CursorMoved: every containment comparison
        // against NaN is false, so early presses hit nothing.
        pointer: (f32::NAN, f32::NAN),
        last_cursor: None,
        started: Instant::now(),
        frames: 0,
        modifiers: ModifiersState::default(),
        ime_allowed: false,
        ime_rect: None,
        error: None,
        watch_wake: false,
        watch_hook,
    };
    if let Some(spawn_watcher) = spawn_watcher {
        spawn_watcher(event_loop.create_proxy());
    }
    let loop_result = event_loop.run_app(&mut app);
    let stats = app.take_result();
    // Explicit clipboard teardown (arboard lifecycle): arboard warns that
    // frameworks owning the event loop can keep the handle from dropping
    // naturally at process exit, and on Wayland/X11 this application is
    // the clipboard owner. Release both strong references here, while the
    // process is still in control.
    if clipboard.is_some() {
        view.set_clipboard_provider(std::rc::Rc::new(velqu_view::NullClipboardProvider));
    }
    drop(clipboard);
    loop_result.map_err(ShellError::from)?;
    stats
}

struct ShellApp<'a> {
    config: ShellConfig,
    view: &'a mut VelquView,
    context: softbuffer::Context<winit::event_loop::OwnedDisplayHandle>,
    window: Option<std::rc::Rc<Window>>,
    surface:
        Option<softbuffer::Surface<winit::event_loop::OwnedDisplayHandle, std::rc::Rc<Window>>>,
    dirty: bool,
    /// Last pointer position in viewport device px (physical window px).
    pointer: (f32, f32),
    /// The cursor icon currently installed on the window, so repeat moves
    /// over the same cursor region cost nothing.
    last_cursor: Option<CursorStyle>,
    started: Instant,
    frames: u64,
    modifiers: ModifiersState,
    /// The IME enablement last pushed to the window, so state changes are
    /// the only `set_ime_allowed` calls (M4c3).
    ime_allowed: bool,
    /// The candidate rect last pushed to the window, deduplicating
    /// `set_ime_cursor_area` calls (M4c3).
    ime_rect: Option<velqu_view::ControlRect>,
    error: Option<ShellError>,
    /// Whether a watch wakeup is pending (M6c).
    watch_wake: bool,
    /// The host's watch hook, run on the main thread per wake.
    watch_hook: Option<WatchHook<'a>>,
}

impl ShellApp<'_> {
    fn take_result(self) -> Result<ShellStats, ShellError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(ShellStats {
            frames: self.frames,
            uptime: self.started.elapsed(),
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: ShellError) {
        self.error = Some(error);
        event_loop.exit();
    }

    /// Creates the window + surface once the loop is live.
    fn setup(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ShellError> {
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.logical_size.0,
                self.config.logical_size.1,
            ))
            .with_resizable(self.config.resizable);
        let window = std::rc::Rc::new(event_loop.create_window(attrs)?);
        let surface = softbuffer::Surface::new(&self.context, window.clone())?;
        window.request_redraw();
        self.window = Some(window);
        self.surface = Some(surface);
        self.dirty = true;
        Ok(())
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The viewport matching the current window size, for input calls.
    /// `None` before the window exists or while minimized (zero-size);
    /// input is silently dropped in both cases.
    fn input_viewport(&self) -> Option<Viewport> {
        let window = self.window.as_ref()?;
        let physical = window.inner_size();
        Viewport::try_new(
            physical.width.max(1),
            physical.height.max(1),
            window.scale_factor() as f32,
        )
        .ok()
    }

    /// Feeds the pointer position to the view (viewport device px are
    /// physical window px). A hover change means interaction-styled
    /// pixels changed: the repaint is presentation-only (no relayout).
    /// The CSS cursor maps onto the platform cursor - a cursor change
    /// alone neither repaints nor lays out.
    fn pointer_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        let Some(viewport) = self.input_viewport() else {
            return;
        };
        self.pointer = (position.x as f32, position.y as f32);
        let hover_changed = self
            .view
            .pointer_move(viewport, self.pointer.0, self.pointer.1);
        if hover_changed {
            self.dirty = true;
        }
        let cursor = self
            .view
            .cursor_under(viewport, self.pointer.0, self.pointer.1);
        if self.last_cursor != Some(cursor) {
            self.last_cursor = Some(cursor);
            if let Some(window) = &self.window {
                window.set_cursor(cursor_icon(cursor));
            }
        }
    }

    /// Pushes IME enablement and the candidate-rect anchor to the window
    /// (M4c3, ADR 0014). Called after anything that can move the caret or
    /// change focus: keyboard/pointer input, wheel scrolling, redraws,
    /// window focus, and IME events themselves. Both calls are
    /// change-gated; the rect is in physical pixels, matching the
    /// renderer's device-space geometry.
    fn update_ime(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let allowed = self.view.wants_ime();
        if allowed != self.ime_allowed {
            self.ime_allowed = allowed;
            window.set_ime_allowed(allowed);
        }
        if !allowed {
            self.ime_rect = None;
            return;
        }
        let Some(viewport) = self.input_viewport() else {
            return;
        };
        let Some(rect) = self.view.ime_cursor_rect(viewport) else {
            return;
        };
        if self.ime_rect != Some(rect) {
            self.ime_rect = Some(rect);
            window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(rect.x.round() as i32, rect.y.round() as i32),
                winit::dpi::PhysicalSize::new(
                    rect.width.round().max(1.0) as u32,
                    rect.height.round().max(1.0) as u32,
                ),
            );
        }
    }

    /// Translates winit's window-level IME events onto the view's
    /// backend-independent composition API (M4c3, ADR 0014).
    fn ime_event(&mut self, event: Ime) {
        let changed = match event {
            Ime::Preedit(text, cursor) => self.view.ime_preedit(&text, cursor),
            Ime::Commit(text) => self.view.ime_commit(&text),
            // The platform IME went away mid-composition.
            Ime::Disabled => self.view.ime_cancel(),
            Ime::Enabled => false,
        };
        if changed {
            self.dirty = true;
            self.request_redraw();
        }
        self.update_ime();
    }

    fn key_modifiers(&self) -> KeyModifiers {
        KeyModifiers {
            ctrl: self.modifiers.control_key(),
            command: self.modifiers.super_key(),
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
        }
    }

    fn keyboard_input(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        let modifiers = self.key_modifiers();
        match route_keyboard_input(
            &event.logical_key,
            event.physical_key,
            modifiers,
            event.text.as_deref(),
        ) {
            KeyboardRoute::Command(KeyCommand::Escape) => event_loop.exit(),
            KeyboardRoute::Command(KeyCommand::Tab) => {
                if !event.repeat {
                    // Focus is runtime presentation state; if focus styling
                    // exists the pixels change: presentation-only repaint.
                    self.view.focus_next();
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::Command(command) => {
                if self.view.key_command(command, modifiers) {
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::InsertText(text) => {
                if self.view.insert_text(text) {
                    self.dirty = true;
                    self.request_redraw();
                }
            }
            KeyboardRoute::Ignored => {}
        }
    }

    /// Renders one frame and presents it. Errors end the loop.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let result = self.redraw_inner();
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn redraw_inner(&mut self) -> Result<(), ShellError> {
        // Reactive turns run before the frame (M5c, ADR 0017), under
        // the M6a ownership model (ADR 0019): drain the queue into a
        // batch, hand the batch to the pump. The shell observes no
        // events, so the batch drops here; anything a turn generates
        // stays queued for the next redraw. A no-op for documents
        // without reactive markup.
        let batch = self.view.take_events();
        self.view.pump_reactive(&batch);
        drop(batch);
        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        let physical = window.inner_size();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(physical.width),
            NonZeroU32::new(physical.height),
        ) else {
            // Minimized to zero-size; nothing to present.
            return Ok(());
        };
        let scale = window.scale_factor() as f32;

        let FrameResult { frame, .. } = self.view.render(
            Viewport::try_new(width.get(), height.get(), scale).map_err(ShellError::from)?,
        )?;

        let surface = self
            .surface
            .as_mut()
            .ok_or_else(|| ShellError::new("redraw before the surface existed"))?;
        surface.resize(width, height)?;
        let mut buffer = surface.buffer_mut()?;
        for (dst, rgba) in buffer.iter_mut().zip(frame.pixels().chunks_exact(4)) {
            // softbuffer's buffer format is native-endian 0x00RRGGBB.
            *dst = u32::from_be_bytes([0, rgba[0], rgba[1], rgba[2]]);
        }
        buffer.present()?;
        self.frames += 1;
        Ok(())
    }
}

impl ApplicationHandler<ShellUserEvent> for ShellApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.setup(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(_) => {
                self.dirty = true;
                self.request_redraw();
            }
            winit::event::WindowEvent::ScaleFactorChanged { .. } => {
                self.dirty = true;
                self.request_redraw();
            }
            winit::event::WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            winit::event::WindowEvent::KeyboardInput { event, .. } => {
                self.keyboard_input(event_loop, &event);
                self.update_ime();
            }
            winit::event::WindowEvent::Ime(event) => {
                self.ime_event(event);
            }
            winit::event::WindowEvent::Focused(focused) => {
                if !focused {
                    // Window blur cancels any active composition (M4c3);
                    // the platform IME loses our context with it.
                    self.view.ime_cancel();
                }
                self.update_ime();
            }
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                self.pointer_moved(position);
            }
            winit::event::WindowEvent::CursorLeft { .. } => {
                self.view.pointer_exit();
            }
            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                let Some(viewport) = self.input_viewport() else {
                    return;
                };
                if button != MouseButton::Left {
                    return;
                }
                let (x, y) = self.pointer;
                let changed = match state {
                    ElementState::Pressed => self.view.pointer_press(viewport, x, y),
                    ElementState::Released => self.view.pointer_release(viewport, x, y),
                };
                if changed {
                    // :active / focus styling may have changed pixels.
                    self.dirty = true;
                    self.request_redraw();
                }
                self.update_ime();
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let Some(viewport) = self.input_viewport() else {
                    return;
                };
                // Browser-signed delta (positive dy = scroll forward).
                // winit's is opposite ("positive = content moves down"),
                // and line deltas scale by the wheel line height.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * WHEEL_LINE_PX, -y * WHEEL_LINE_PX),
                    MouseScrollDelta::PixelDelta(position) => {
                        (-(position.x as f32), -(position.y as f32))
                    }
                };
                let (px, py) = self.pointer;
                self.view.wheel(viewport, px, py, dx, dy);
                // The new offset changes pixels: schedule the paint, which
                // re-renders from the baked offsets with zero relayout.
                self.dirty = true;
                self.request_redraw();
                // Ancestor scrolling moves the candidate anchor (M4c3).
                self.update_ime();
            }
            winit::event::WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
                self.dirty = false;
                // Caret geometry may have moved (scroll-to-caret etc.).
                self.update_ime();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(exit_after) = self.config.exit_after {
            if self.started.elapsed() >= exit_after {
                event_loop.exit();
            }
        }
        // A watch wakeup (M6c): the host's hook runs here, on the main
        // thread, with the view — watcher threads never touch it.
        // Only a published reload repaints the document.
        if self.watch_wake {
            self.watch_wake = false;
            if let Some(hook) = self.watch_hook.as_mut() {
                let repaint = (hook)(self.view);
                if repaint {
                    self.dirty = true;
                }
            }
        }
        if self.dirty {
            self.request_redraw();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ShellUserEvent) {
        match event {
            ShellUserEvent::Watch => self.watch_wake = true,
        }
    }
}

/// The routing decision for one keyboard input (pure, testable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardRoute<'a> {
    /// A named key or shortcut chord; dispatch as a command.
    Command(KeyCommand),
    /// Insert the platform's produced text verbatim.
    InsertText(&'a str),
    /// Nothing to do (key releases, textless keys, suppressed chords).
    Ignored,
}

/// Routes one keyboard input (M4c2, ADR 0013):
///
/// 1. Named editing/navigation keys are commands before any text
///    consideration (winit gives Enter the text `"\r"` and Tab `"\t"`).
/// 2. An exact shortcut chord — Ctrl/Cmd + A/C/X/V — is a command.
/// 3. A primary shortcut modifier (Ctrl/Cmd) **without Alt** suppresses
///    text: a modified non-shortcut key must not leak its glyph.
/// 4. Everything else — plain keys, Shift chords, and AltGr-like
///    Ctrl+Alt chords — defers to `KeyEvent.text`, the platform's
///    actually-produced text.
///
/// The Alt rule is the international-layout contract: winit 0.30 has no
/// distinct AltGr flag (AltGr arrives as Ctrl+Alt, e.g. `@` = Ctrl+Alt+Q
/// on German layouts), so Alt disqualifies both the chord match and the
/// suppression, and the produced text routes.
fn route_keyboard_input<'a>(
    logical_key: &Key,
    physical_key: PhysicalKey,
    modifiers: KeyModifiers,
    text: Option<&'a str>,
) -> KeyboardRoute<'a> {
    if let Some(command) = named_key_command(logical_key) {
        return KeyboardRoute::Command(command);
    }
    if let Some(command) = shortcut_chord(logical_key, physical_key, modifiers) {
        return KeyboardRoute::Command(command);
    }
    if (modifiers.ctrl || modifiers.command) && !modifiers.alt {
        return KeyboardRoute::Ignored;
    }
    match text {
        Some(text) if !text.is_empty() => KeyboardRoute::InsertText(text),
        _ => KeyboardRoute::Ignored,
    }
}

/// Maps named editing/navigation keys regardless of modifiers.
fn named_key_command(logical_key: &Key) -> Option<KeyCommand> {
    match logical_key {
        Key::Named(NamedKey::Backspace) => Some(KeyCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(KeyCommand::Delete),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyCommand::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCommand::Right),
        Key::Named(NamedKey::ArrowUp) => Some(KeyCommand::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyCommand::Down),
        Key::Named(NamedKey::Home) => Some(KeyCommand::Home),
        Key::Named(NamedKey::End) => Some(KeyCommand::End),
        Key::Named(NamedKey::Enter) => Some(KeyCommand::Enter),
        Key::Named(NamedKey::Tab) => Some(KeyCommand::Tab),
        Key::Named(NamedKey::Escape) => Some(KeyCommand::Escape),
        _ => None,
    }
}

/// Matches an exact supported shortcut chord: Ctrl/Cmd + A/C/X/V. Letters
/// match the logical key when the layout labels them and fall back to the
/// physical key otherwise; **Alt disqualifies** — a Ctrl+Alt chord is
/// AltGr territory, never a shortcut.
fn shortcut_chord(
    logical_key: &Key,
    physical_key: PhysicalKey,
    modifiers: KeyModifiers,
) -> Option<KeyCommand> {
    if (!modifiers.ctrl && !modifiers.command) || modifiers.alt {
        return None;
    }
    let letter = |target: &str, code: KeyCode| {
        matches!(logical_key, Key::Character(text) if text.eq_ignore_ascii_case(target))
            || matches!(physical_key, PhysicalKey::Code(actual) if actual == code)
    };
    if letter("a", KeyCode::KeyA) {
        Some(KeyCommand::SelectAll)
    } else if letter("c", KeyCode::KeyC) {
        Some(KeyCommand::Copy)
    } else if letter("x", KeyCode::KeyX) {
        Some(KeyCommand::Cut)
    } else if letter("v", KeyCode::KeyV) {
        Some(KeyCommand::Paste)
    } else {
        None
    }
}

/// OS clipboard adapter (M4c2): the only piece of the shell that touches
/// the platform clipboard. arboard's handle needs `&mut` for read/write,
/// so it sits behind a `RefCell` (the shell is single-threaded); the
/// renderer-facing trait stays shared, like `AssetResolver`. Platform
/// failures (contention, non-text payload, unsupported environment)
/// surface as `Err` so the renderer's transactional rules apply — most
/// importantly, a failed cut write keeps the selection.
struct ArboardClipboard {
    board: std::cell::RefCell<arboard::Clipboard>,
}

impl velqu_view::ClipboardProvider for ArboardClipboard {
    fn read(&self) -> Result<Option<String>, velqu_view::ClipboardError> {
        self.board
            .borrow_mut()
            .get_text()
            .map(Some)
            .map_err(|error| velqu_view::ClipboardError::new(error.to_string()))
    }

    fn write(&self, text: &str) -> Result<(), velqu_view::ClipboardError> {
        self.board
            .borrow_mut()
            .set_text(text.to_owned())
            .map_err(|error| velqu_view::ClipboardError::new(error.to_string()))
    }
}

/// Maps the CSS `cursor` value onto the platform cursor (M4b). `Auto`
/// means "the UA decides" - the shell's UA default is the plain arrow.
fn cursor_icon(style: CursorStyle) -> winit::window::CursorIcon {
    match style {
        CursorStyle::Auto | CursorStyle::Default => winit::window::CursorIcon::Default,
        CursorStyle::Pointer => winit::window::CursorIcon::Pointer,
        CursorStyle::Text => winit::window::CursorIcon::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_enter_is_a_command_even_when_winit_supplies_carriage_return() {
        // winit's KeyEvent.text for Enter is Some("\r"); the route must
        // be the command, never the text.
        assert_eq!(
            route_keyboard_input(
                &Key::Named(NamedKey::Enter),
                PhysicalKey::Code(KeyCode::Enter),
                KeyModifiers::default(),
                Some("\r"),
            ),
            KeyboardRoute::Command(KeyCommand::Enter)
        );
    }

    #[test]
    fn tab_and_escape_are_commands_not_text() {
        assert_eq!(
            route_keyboard_input(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                KeyModifiers::default(),
                Some("\t"),
            ),
            KeyboardRoute::Command(KeyCommand::Tab)
        );
        assert_eq!(
            route_keyboard_input(
                &Key::Named(NamedKey::Escape),
                PhysicalKey::Code(KeyCode::Escape),
                KeyModifiers::default(),
                Some("\u{1b}"),
            ),
            KeyboardRoute::Command(KeyCommand::Escape)
        );
    }

    #[test]
    fn control_or_command_a_maps_to_select_all() {
        let logical = Key::Character("a".into());
        assert_eq!(
            shortcut_chord(
                &logical,
                PhysicalKey::Code(KeyCode::KeyA),
                KeyModifiers {
                    ctrl: true,
                    ..KeyModifiers::default()
                }
            ),
            Some(KeyCommand::SelectAll)
        );
        assert_eq!(
            shortcut_chord(
                &logical,
                PhysicalKey::Code(KeyCode::KeyA),
                KeyModifiers {
                    command: true,
                    ..KeyModifiers::default()
                }
            ),
            Some(KeyCommand::SelectAll)
        );
        // Alt disqualifies the chord...
        assert_eq!(
            shortcut_chord(
                &logical,
                PhysicalKey::Code(KeyCode::KeyA),
                KeyModifiers {
                    alt: true,
                    ..KeyModifiers::default()
                }
            ),
            None
        );
        // ...and a textless Ctrl+Alt+A routes nowhere (an AltGr layout
        // that did produce text would route that text instead).
        assert_eq!(
            route_keyboard_input(
                &logical,
                PhysicalKey::Code(KeyCode::KeyA),
                KeyModifiers {
                    ctrl: true,
                    alt: true,
                    ..KeyModifiers::default()
                },
                None,
            ),
            KeyboardRoute::Ignored
        );
    }

    #[test]
    fn plain_character_keys_route_their_produced_text() {
        assert_eq!(
            route_keyboard_input(
                &Key::Character("é".into()),
                PhysicalKey::Code(KeyCode::KeyE),
                KeyModifiers::default(),
                Some("é"),
            ),
            KeyboardRoute::InsertText("é")
        );
        // Textless plain keys (function keys etc.) do nothing.
        assert_eq!(
            route_keyboard_input(
                &Key::Character("é".into()),
                PhysicalKey::Code(KeyCode::KeyE),
                KeyModifiers::default(),
                None,
            ),
            KeyboardRoute::Ignored
        );
    }

    #[test]
    fn clipboard_shortcuts_map_to_commands() {
        let ctrl = KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        };
        // By logical key…
        assert_eq!(
            shortcut_chord(
                &Key::Character("c".into()),
                PhysicalKey::Code(KeyCode::KeyC),
                ctrl
            ),
            Some(KeyCommand::Copy)
        );
        assert_eq!(
            shortcut_chord(
                &Key::Character("X".into()),
                PhysicalKey::Code(KeyCode::KeyX),
                ctrl
            ),
            Some(KeyCommand::Cut)
        );
        // …and by physical key when the layout labels it differently.
        assert_eq!(
            shortcut_chord(
                &Key::Character("û".into()),
                PhysicalKey::Code(KeyCode::KeyV),
                ctrl
            ),
            Some(KeyCommand::Paste)
        );
        // Cmd on macOS behaves like Ctrl.
        assert_eq!(
            shortcut_chord(
                &Key::Character("c".into()),
                PhysicalKey::Code(KeyCode::KeyC),
                KeyModifiers {
                    command: true,
                    ..KeyModifiers::default()
                }
            ),
            Some(KeyCommand::Copy)
        );
        // Without Ctrl/Cmd the letters stay plain text.
        assert_eq!(
            shortcut_chord(
                &Key::Character("c".into()),
                PhysicalKey::Code(KeyCode::KeyC),
                KeyModifiers::default()
            ),
            None
        );
    }

    #[test]
    fn modified_non_shortcut_keys_never_become_text() {
        // Ctrl held + a non-shortcut key: no command and, critically, no
        // fall-through that would insert the key's text.
        assert_eq!(
            route_keyboard_input(
                &Key::Character("k".into()),
                PhysicalKey::Code(KeyCode::KeyK),
                KeyModifiers {
                    ctrl: true,
                    ..KeyModifiers::default()
                },
                Some("k"),
            ),
            KeyboardRoute::Ignored
        );
    }

    #[test]
    fn altgr_chords_route_produced_text_instead_of_shortcuts() {
        // German-layout-like: @ = Ctrl+Alt+Q. The physical key would
        // alias the Copy chord, and the logical key may come through as
        // either the base letter or the produced glyph — either way the
        // produced text wins and nothing is copied.
        let modifiers = KeyModifiers {
            ctrl: true,
            alt: true,
            ..KeyModifiers::default()
        };
        assert_eq!(
            route_keyboard_input(
                &Key::Character("q".into()),
                PhysicalKey::Code(KeyCode::KeyQ),
                modifiers,
                Some("@"),
            ),
            KeyboardRoute::InsertText("@")
        );
        assert_eq!(
            route_keyboard_input(
                &Key::Character("@".into()),
                PhysicalKey::Code(KeyCode::KeyQ),
                modifiers,
                Some("@"),
            ),
            KeyboardRoute::InsertText("@")
        );
        // The chord matcher itself never fires under Alt.
        assert_eq!(
            shortcut_chord(
                &Key::Character("q".into()),
                PhysicalKey::Code(KeyCode::KeyQ),
                modifiers
            ),
            None
        );
    }
}
