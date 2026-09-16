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
//! Input is forwarded to renderer hot paths only (M1: redraw scheduling on
//! resize/DPI change, Escape to close). Real event dispatch, focus, and IME
//! land in M4 and will stay in Rust, never routed through a JS runtime.

use std::fmt;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use velqu_view::{FrameResult, VelquError, VelquView, Viewport};
use winit::application::ApplicationHandler;
use winit::event::ElementState;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

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

/// Opens a native window and runs the event loop until the window closes.
///
/// The window shows live [`VelquView`] frames, re-rendering whenever the
/// window is resized or its DPI scale changes. Blocking; returns when the
/// user closes the window, `exit_after` elapses, or an error occurs.
pub fn run(config: ShellConfig, view: &mut VelquView) -> Result<ShellStats, ShellError> {
    let event_loop = EventLoop::new()?;
    let context = softbuffer::Context::new(event_loop.owned_display_handle())?;

    // `Wait` sleeps until input; a deadline (smoke tests) needs `WaitUntil`
    // so the loop wakes up to honor `exit_after` with no other events.
    event_loop.set_control_flow(match config.exit_after {
        Some(exit_after) => winit::event_loop::ControlFlow::WaitUntil(Instant::now() + exit_after),
        None => winit::event_loop::ControlFlow::Wait,
    });

    let mut app = ShellApp {
        config,
        view,
        context,
        window: None,
        surface: None,
        dirty: true,
        started: Instant::now(),
        frames: 0,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.take_result()
}

struct ShellApp<'a> {
    config: ShellConfig,
    view: &'a mut VelquView,
    context: softbuffer::Context<winit::event_loop::OwnedDisplayHandle>,
    window: Option<std::rc::Rc<Window>>,
    surface:
        Option<softbuffer::Surface<winit::event_loop::OwnedDisplayHandle, std::rc::Rc<Window>>>,
    dirty: bool,
    started: Instant,
    frames: u64,
    error: Option<ShellError>,
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

    /// Renders one frame and presents it. Errors end the loop.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let result = self.redraw_inner();
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn redraw_inner(&mut self) -> Result<(), ShellError> {
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

        let FrameResult { frame, .. } =
            self.view
                .render(Viewport::new(width.get(), height.get(), scale))?;

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

impl ApplicationHandler for ShellApp<'_> {
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
            winit::event::WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                {
                    event_loop.exit();
                }
            }
            winit::event::WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
                self.dirty = false;
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
        if self.dirty {
            self.request_redraw();
        }
    }
}
