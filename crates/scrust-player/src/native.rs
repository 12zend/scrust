//! Native host: winit window, softbuffer presentation, keyboard / mouse input.

use crate::audio::RodioAudio;
use scrust_render::SoftRenderer;
use scrust_runtime::cli::{install_native_clock, parse_args, Args};
use scrust_runtime::{set_world_ptr, Options, ProgramDef, World};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

pub fn main(program: ProgramDef, defaults: Options) {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (args, rest) = parse_args(defaults, &argv);
    let program: &'static ProgramDef = Box::leak(Box::new(program));
    let mut scale = 2.0;
    let mut screenshot: Option<String> = None;
    let mut embed = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scale" => scale = it.next().and_then(|v| v.parse().ok()).unwrap_or(2.0),
            "--screenshot" => screenshot = it.next().cloned(),
            "--embed" => embed = true,
            _ => {}
        }
    }
    if embed {
        crate::embed::main(program, args, scale);
        return;
    }
    if args.trace.is_some() || screenshot.is_some() || args.frames.is_some() {
        offscreen(program, args, scale, screenshot);
        return;
    }
    let mut world = World::new(program, args.options.clone());
    install_native_clock(&mut world);
    world.current_msecs = world.clock_ms();
    world.timer_start = world.current_msecs;
    world.audio = RodioAudio::new(program).map(|a| Box::new(a) as Box<dyn scrust_runtime::Audio>);
    let step = Duration::from_secs_f64(1.0 / args.options.fps);
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { program, world, scale, window: None, surface: None, context: None, step, next_tick: Instant::now(), mouse: (0.0, 0.0), started: false };
    let _ = event_loop.run_app(&mut app);
}

/// Headless run with the renderer attached (`--frames`, `--trace`, `--screenshot`).
fn offscreen(program: &'static ProgramDef, args: Args, scale: f64, screenshot: Option<String>) {
    let mut world = World::new(program, args.options.clone());
    if !args.options.deterministic {
        install_native_clock(&mut world);
        world.current_msecs = world.clock_ms();
        world.timer_start = world.current_msecs;
    }
    // Deterministic runs draw on the CPU so that pen pixels (and with them
    // `touching color`) do not depend on the GPU, unless Metal is asked for.
    let metal = std::env::var("SCRUST_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("metal"));
    let renderer = if args.options.deterministic && !metal { SoftRenderer::new_cpu(program, scale) } else { SoftRenderer::new(program, scale) };
    world.renderer = Some(Box::new(renderer));
    let frames = args.trace.or(args.frames).unwrap_or(1);
    let trace = args.trace.is_some();
    scrust_runtime::run_headless(&mut world, frames, |w| {
        if trace {
            println!("{}", w.trace_line());
        }
    });
    if let Some(path) = screenshot {
        let mut r = world.renderer.take().expect("renderer");
        if let Some(sr) = r.as_any_mut().downcast_mut::<SoftRenderer>() {
            sr.draw(&world);
            let mut rgba = Vec::with_capacity(sr.frame.len() * 4);
            for p in &sr.frame {
                rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]);
            }
            if let Err(e) = image::save_buffer(&path, &rgba, sr.width as u32, sr.height as u32, image::ColorType::Rgba8) {
                eprintln!("cannot write {path}: {e}");
            }
        }
        world.renderer = Some(r);
    }
    if args.bench {
        eprintln!("frames: {frames}");
    }
}

struct App {
    program: &'static ProgramDef,
    world: Box<World>,
    scale: f64,
    window: Option<Rc<Window>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    step: Duration,
    next_tick: Instant,
    /// Mouse position in Scratch coordinates.
    mouse: (f64, f64),
    started: bool,
}

impl App {
    fn present(&mut self) {
        let (Some(window), Some(_)) = (self.window.clone(), self.surface.as_ref()) else { return };
        // Draw with the renderer detached from the world (it reads the world).
        let Some(mut r) = self.world.renderer.take() else { return };
        let mut frame: Option<(Vec<u32>, usize, usize)> = None;
        if let Some(sr) = r.as_any_mut().downcast_mut::<SoftRenderer>() {
            sr.draw(&self.world);
            frame = Some((std::mem::take(&mut sr.frame), sr.width, sr.height));
        }
        if let Some((pixels, fw, fh)) = frame {
            let size = window.inner_size();
            let (w, h) = (size.width.max(1), size.height.max(1));
            if let Some(surface) = self.surface.as_mut() {
                if surface.resize(NonZeroU32::new(w).expect("width"), NonZeroU32::new(h).expect("height")).is_ok() {
                    if let Ok(mut buf) = surface.buffer_mut() {
                        if w as usize == fw && h as usize == fh {
                            buf.copy_from_slice(&pixels);
                        } else {
                            // Nearest-neighbour stretch to the window size.
                            for y in 0..h as usize {
                                let sy = y * fh / h as usize;
                                let row = &pixels[sy * fw..(sy + 1) * fw];
                                let out = &mut buf[y * w as usize..(y + 1) * w as usize];
                                for (x, o) in out.iter_mut().enumerate() {
                                    *o = row[x * fw / w as usize];
                                }
                            }
                        }
                        let _ = buf.present();
                    }
                }
            }
            if let Some(sr) = r.as_any_mut().downcast_mut::<SoftRenderer>() {
                sr.frame = pixels;
            }
        }
        self.world.renderer = Some(r);
    }

    fn to_scratch(&self, pos: PhysicalPosition<f64>) -> (f64, f64) {
        let Some(window) = &self.window else { return (0.0, 0.0) };
        let size = window.inner_size();
        let x = 480.0 * (pos.x / size.width.max(1) as f64 - 0.5);
        let y = -360.0 * (pos.y / size.height.max(1) as f64 - 0.5);
        (x, y)
    }

    fn key(&mut self, key: &Key, text: Option<&str>, down: bool) {
        let name: String = match key {
            Key::Named(NamedKey::Space) => " ".into(),
            Key::Named(NamedKey::ArrowLeft) => "ArrowLeft".into(),
            Key::Named(NamedKey::ArrowRight) => "ArrowRight".into(),
            Key::Named(NamedKey::ArrowUp) => "ArrowUp".into(),
            Key::Named(NamedKey::ArrowDown) => "ArrowDown".into(),
            Key::Named(NamedKey::Enter) => "Enter".into(),
            Key::Named(NamedKey::Backspace) => "Backspace".into(),
            Key::Named(NamedKey::Escape) => "Escape".into(),
            Key::Named(NamedKey::F5) => "F5".into(),
            Key::Character(s) => s.to_string(),
            _ => return,
        };
        crate::input::key_event(&mut self.world, &name, text, down);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("scrust").with_inner_size(LogicalSize::new(480.0 * self.scale, 360.0 * self.scale));
        let window = Rc::new(event_loop.create_window(attrs).expect("window"));
        let context = softbuffer::Context::new(window.clone()).expect("display context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        // Render at the window's physical resolution.
        let physical = window.inner_size();
        let render_scale = (physical.width as f64 / 480.0).max(1.0);
        self.world.renderer = Some(Box::new(SoftRenderer::new(self.program, render_scale)));
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
        set_world_ptr(&mut *self.world);
        if !self.started {
            self.started = true;
            self.world.green_flag();
        }
        self.next_tick = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        set_world_ptr(&mut *self.world);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let text = event.text.as_ref().map(|t| t.to_string());
                self.key(&event.logical_key, text.as_deref(), down);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = self.to_scratch(position);
                self.world.post_mouse_move(self.mouse.0, self.mouse.1);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let (x, y) = self.mouse;
                self.world.post_mouse_button(x, y, state == ElementState::Pressed);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_tick {
            let wp: *mut World = &mut *self.world;
            // SAFETY: the world is only accessed through `wp` during the step.
            unsafe { scrust_runtime::step(wp) };
            self.next_tick += self.step;
            // Do not try to catch up after a long stall.
            if self.next_tick < now {
                self.next_tick = now + self.step;
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
    }
}
