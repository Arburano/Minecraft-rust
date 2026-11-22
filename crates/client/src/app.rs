//! The window, the frame loop, input, and the switch between the menu and a
//! running world.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use client::session::{Session, SessionConfig};
use client::settings::Settings;
use render::selection::select_for_surface;
use render::terrain::renderer::{DEPTH_FORMAT, TerrainPath};
use sim::physics::PlayerInput;
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::menu::{self, Menu, MenuAction, PauseAction, TITLE};
use crate::view::{SKY, WorldView};

const TICK: Duration = Duration::from_millis(50);

/// Command-line options the app needs.
pub struct Options {
    /// Settings, worlds and caches.
    pub data_dir: PathBuf,
    /// The `core` mod's data.
    pub mods_dir: PathBuf,
    /// Offline name from the command line (overrides the saved one).
    pub offline_name: Option<String>,
    /// Print the time to the first menu frame and exit.
    pub measure_startup: bool,
    /// Process start, for the startup measurement.
    pub start: Instant,
    /// Open (or create) this world right away (testing and screenshots).
    pub autoplay: Option<String>,
    /// Save a screenshot after this many frames in the current state, then exit.
    pub screenshot: Option<(PathBuf, u32)>,
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    path: TerrainPath,
    egui: egui_wgpu::Renderer,
}

fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

#[derive(Default)]
struct Keys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    jump: bool,
    sprint: bool,
    descend: bool,
    /// Time of the last jump press, for the double tap that toggles flight.
    last_jump: Option<Instant>,
    /// A double tap waiting for the next tick.
    toggle_fly: bool,
}

/// Two jump presses closer than this toggle flight.
const DOUBLE_TAP: Duration = Duration::from_millis(300);

impl Keys {
    fn input(&self) -> PlayerInput {
        let axis = |p: bool, n: bool| f64::from(u8::from(p)) - f64::from(u8::from(n));
        PlayerInput {
            forward: axis(self.forward, self.back),
            strafe: axis(self.right, self.left),
            jump: self.jump,
            sprint: self.sprint,
            descend: self.descend,
            toggle_fly: self.toggle_fly,
            fly_speed: 0.0,
            yaw: 0.0,
        }
    }
}

struct Game {
    session: Session,
    view: WorldView,
    keys: Keys,
    paused: bool,
    acc: Duration,
    hotbar: Vec<(world::BlockStateId, &'static str)>,
    selected: usize,
    debug: bool,
    frame_ms: f32,
}

const HOTBAR: [&str; 9] = [
    "stone",
    "dirt",
    "grass_block",
    "cobblestone",
    "oak_planks",
    "oak_log",
    "glass",
    "torch",
    "glowstone",
];

/// The application.
pub struct App {
    opts: Options,
    settings: Settings,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui_state: Option<egui_winit::State>,
    egui_ctx: egui::Context,
    menu: Menu,
    game: Option<Game>,
    last_frame: Instant,
    frames: u32,
    first_frame_done: bool,
    grabbed: bool,
    textures_line: String,
}

impl App {
    /// A new app with persisted settings.
    pub fn new(opts: Options) -> Self {
        let (mut settings, error) = Settings::load(&opts.data_dir.join("settings.toml"));
        if let Some(n) = &opts.offline_name {
            settings.offline_name = Some(n.clone());
        }
        let mut menu = Menu::new(opts.data_dir.join("worlds"), &settings);
        menu.message = error.map(|e| format!("Settings were damaged and reset: {e}"));
        Self {
            opts,
            settings,
            window: None,
            gpu: None,
            egui_state: None,
            egui_ctx: egui::Context::default(),
            menu,
            game: None,
            last_frame: Instant::now(),
            frames: 0,
            first_frame_done: false,
            grabbed: false,
            textures_line: String::new(),
        }
    }

    fn save_settings(&mut self) {
        let _ = std::fs::create_dir_all(&self.opts.data_dir);
        if let Err(e) = self
            .settings
            .save(&self.opts.data_dir.join("settings.toml"))
        {
            self.menu.message = Some(format!("Could not save settings: {e}"));
        }
    }

    fn init_gpu(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
    ) -> Result<Gpu, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| format!("no surface: {e}"))?;
        let selected = select_for_surface(
            &instance,
            &surface,
            &self.settings.gpu,
            &wgpu::DeviceDescriptor {
                label: Some("client"),
                ..Default::default()
            },
        )
        .map_err(|failed| format!("no usable GPU ({} adapters failed)", failed.len()))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&selected.adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface does not fit the adapter")?;
        let caps = surface.get_capabilities(&selected.adapter);
        if let Some(f) = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = f;
        }
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&selected.device, &config);
        self.menu.gpu_line = format!(
            "GPU: {} ({:?}), path {:?}",
            selected.candidate.name,
            selected.candidate.backend,
            TerrainPath::default_for(selected.adapter.get_downlevel_capabilities().flags)
        );
        let egui = egui_wgpu::Renderer::new(
            &selected.device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        Ok(Gpu {
            depth: depth_view(&selected.device, config.width, config.height),
            path: TerrainPath::default_for(selected.adapter.get_downlevel_capabilities().flags),
            surface,
            device: selected.device,
            queue: selected.queue,
            config,
            egui,
        })
    }

    fn open_world(&mut self, dir: PathBuf) {
        let Some(gpu) = &self.gpu else { return };
        let mut config = SessionConfig::new(self.opts.mods_dir.clone());
        config.radius = self.settings.quality.settings().render_distance as i32;
        config.threads = self.settings.threads();
        config.autosave = Duration::from_secs(u64::from(self.settings.autosave_seconds.max(5)));
        match Session::open(&dir, config) {
            Ok(session) => {
                let (textures, line) =
                    block_textures(&self.settings, &self.opts.mods_dir, session.registry());
                self.textures_line = line;
                let view = WorldView::new(
                    &gpu.device,
                    &gpu.queue,
                    gpu.path,
                    gpu.config.format,
                    &textures,
                );
                let hotbar = HOTBAR
                    .iter()
                    .filter_map(|n| {
                        session
                            .registry()
                            .state_id(&format!("core:{n}"))
                            .map(|id| (id, *n))
                    })
                    .collect();
                self.game = Some(Game {
                    session,
                    view,
                    keys: Keys::default(),
                    paused: false,
                    acc: Duration::ZERO,
                    hotbar,
                    selected: 0,
                    debug: false,
                    frame_ms: 16.7,
                });
                self.set_grab(true);
            }
            Err(e) => self.menu.message = Some(format!("Could not open the world: {e}")),
        }
    }

    fn close_world(&mut self) {
        if let Some(game) = self.game.take()
            && let Err(e) = game.session.close()
        {
            self.menu.message = Some(format!("Saving failed: {e}"));
        }
        self.set_grab(false);
        self.menu.refresh();
        self.menu.screen = menu::Screen::Worlds;
    }

    fn set_grab(&mut self, grab: bool) {
        let Some(w) = &self.window else { return };
        if grab {
            let ok = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            w.set_cursor_visible(false);
            self.grabbed = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    fn playing(&self) -> bool {
        self.game.as_ref().is_some_and(|g| !g.paused)
    }

    fn game_key(&mut self, code: KeyCode, down: bool) {
        let Some(game) = &mut self.game else { return };
        match code {
            KeyCode::KeyW => game.keys.forward = down,
            KeyCode::KeyS => game.keys.back = down,
            KeyCode::KeyA => game.keys.left = down,
            KeyCode::KeyD => game.keys.right = down,
            KeyCode::Space => {
                if down {
                    let now = Instant::now();
                    if game.keys.last_jump.is_some_and(|t| now - t < DOUBLE_TAP) {
                        game.keys.toggle_fly = true;
                        game.keys.last_jump = None;
                    } else {
                        game.keys.last_jump = Some(now);
                    }
                }
                game.keys.jump = down;
            }
            KeyCode::ControlLeft => game.keys.sprint = down,
            KeyCode::ShiftLeft => game.keys.descend = down,
            KeyCode::F3 if down => game.debug = !game.debug,
            KeyCode::Escape if down => {
                game.paused = !game.paused;
                game.keys = Keys::default();
                let grab = !game.paused;
                self.set_grab(grab);
            }
            _ if down => {
                let digit = [
                    KeyCode::Digit1,
                    KeyCode::Digit2,
                    KeyCode::Digit3,
                    KeyCode::Digit4,
                    KeyCode::Digit5,
                    KeyCode::Digit6,
                    KeyCode::Digit7,
                    KeyCode::Digit8,
                    KeyCode::Digit9,
                ]
                .iter()
                .position(|&k| k == code);
                if let Some(i) = digit
                    && i < game.hotbar.len()
                {
                    game.selected = i;
                }
            }
            _ => {}
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        let (Some(window), Some(_)) = (self.window.clone(), self.gpu.as_ref()) else {
            return;
        };

        // Simulation.
        let mut failed = None;
        if let Some(game) = &mut self.game {
            game.frame_ms = game.frame_ms * 0.95 + dt.as_secs_f32() * 1000.0 * 0.05;
            if let Err(e) = game.session.update() {
                failed = Some(e.to_string());
            }
            if !game.paused {
                game.acc = (game.acc + dt).min(TICK * 5);
                while game.acc >= TICK {
                    // The flight toggle is a one-tick event.
                    let input = game.keys.input();
                    game.keys.toggle_fly = false;
                    game.session.fixed_tick(&input);
                    game.acc -= TICK;
                }
            }
        }
        if let Some(e) = failed {
            self.close_world();
            self.menu.message = Some(format!("The world stopped: {e}"));
        }

        // UI.
        let Some(egui_state) = &mut self.egui_state else {
            return;
        };
        let raw = egui_state.take_egui_input(&window);
        let mut menu_action = MenuAction::None;
        let mut pause_action = PauseAction::None;
        let (menu, settings, game) = (&mut self.menu, &self.settings, &self.game);
        let textures_line = &self.textures_line;
        let full = self.egui_ctx.run_ui(raw, |ui| match game {
            None => menu_action = menu.show(ui, settings),
            Some(g) => {
                let names: Vec<&str> = g.hotbar.iter().map(|h| h.1).collect();
                let debug = g
                    .debug
                    .then(|| format!("{}\n{textures_line}", debug_text(g, &menu.gpu_line)));
                let status = g
                    .session
                    .is_flying()
                    .then(|| format!("Flying — speed ×{:.2} (mouse wheel)", g.session.fly_speed()));
                menu::hud(
                    ui,
                    &names,
                    g.selected,
                    debug.as_deref(),
                    status.as_deref(),
                    !g.session.is_playing(),
                );
                if g.paused {
                    pause_action = menu::pause(ui);
                }
            }
        });
        egui_state.handle_platform_output(&window, full.platform_output);

        // Render.
        let Some(gpu) = &mut self.gpu else { return };
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
        if let Some(game) = &mut self.game {
            game.view.sync(&gpu.queue, &mut game.session);
            let alpha = game.acc.as_secs_f64() / TICK.as_secs_f64();
            let rs = self.settings.quality.settings();
            game.view.prepare(
                &gpu.queue,
                &game.session,
                &rs,
                self.settings.fov_degrees,
                aspect,
                alpha,
            );
        }
        let clear = if self.game.is_some() {
            wgpu::Color {
                r: f64::from(SKY[0]),
                g: f64::from(SKY[1]),
                b: f64::from(SKY[2]),
                a: 1.0,
            }
        } else {
            wgpu::Color::BLACK
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if let Some(game) = &self.game {
                game.view.render(&mut pass);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.config.width, gpu.config.height],
            pixels_per_point: full.pixels_per_point,
        };
        let jobs = self.egui_ctx.tessellate(full.shapes, full.pixels_per_point);
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                gpu.egui.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let extra = gpu
            .egui
            .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            gpu.egui.render(&mut pass, &jobs, &screen);
        }
        for id in &full.textures_delta.free {
            gpu.egui.free_texture(id);
        }
        self.frames += 1;
        let shot = self
            .opts
            .screenshot
            .as_ref()
            .filter(|(_, n)| {
                self.frames >= *n && self.game.as_ref().is_none_or(|g| g.session.is_playing())
            })
            .map(|(p, _)| p.clone());
        let readback = shot
            .as_ref()
            .map(|_| copy_frame(gpu, &mut encoder, &frame.texture));
        gpu.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        if let (Some(path), Some(buf)) = (shot, readback) {
            save_frame(gpu, &buf, &path);
            event_loop.exit();
        }
        window.pre_present_notify();
        gpu.queue.present(frame);

        if !self.first_frame_done {
            self.first_frame_done = true;
            let ms = self.opts.start.elapsed().as_secs_f64() * 1000.0;
            println!("startup: first menu frame after {ms:.0} ms");
            if self.opts.measure_startup {
                event_loop.exit();
            }
        }

        // Actions.
        match menu_action {
            MenuAction::None => {}
            MenuAction::Play(dir) => {
                self.menu.message = None;
                self.open_world(dir);
            }
            MenuAction::SaveSettings(s) => {
                self.settings = s;
                self.save_settings();
            }
            MenuAction::Quit => event_loop.exit(),
        }
        match pause_action {
            PauseAction::None => {}
            PauseAction::Resume => {
                if let Some(g) = &mut self.game {
                    g.paused = false;
                }
                self.set_grab(true);
            }
            PauseAction::SaveAndQuit => self.close_world(),
        }
    }
}

/// Builds the world's block textures: from the player's own installation if
/// enabled and found, fallback textures for everything else. Returns a status
/// line for the debug overlay.
fn block_textures(
    settings: &Settings,
    mods_dir: &std::path::Path,
    registry: &world::Registry,
) -> (client::textures::BlockTextures, String) {
    let faces = client::textures::read_block_faces(&mods_dir.join("blocks"));
    let mut status = String::from("textures: fallback");
    let mut installed = None;
    if settings.own_assets.enabled {
        match settings.own_assets.resolve_folder() {
            None => status = "textures: fallback (no own installation found)".into(),
            Some(dir) => match assets::local_install::InstalledTextures::open(
                &dir,
                settings.own_assets.version.as_deref(),
            ) {
                Ok(t) => installed = Some(t),
                Err(e) => status = format!("textures: fallback ({e})"),
            },
        }
    }
    let t0 = Instant::now();
    let version = installed.as_ref().map(|t| t.version.clone());
    let built = client::textures::build(registry, &faces, installed.as_mut());
    if let Some(v) = version {
        status = format!(
            "textures: own installation {v}, {} faces (+{} fallback), {:.0} ms",
            built.from_installation,
            built.fallback,
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }
    (built, status)
}

fn debug_text(g: &Game, gpu_line: &str) -> String {
    let p = g.session.player().pos;
    let l = g.session.loader().stats();
    format!(
        "{:.1} ms/frame ({:.0} FPS)\nx {:.2}  y {:.2}  z {:.2}\nloaded columns {}  meshes {}  jobs {}\nupload errors {}\n{gpu_line}",
        g.frame_ms,
        1000.0 / g.frame_ms.max(0.1),
        p.x,
        p.y,
        p.z,
        g.session.map().len(),
        g.session.loader().meshes().len(),
        l.submitted,
        g.view.upload_errors(),
    )
}

/// Copies the frame for a screenshot (the surface allows `COPY_SRC`).
fn copy_frame(gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, tex: &wgpu::Texture) -> wgpu::Buffer {
    let (w, h) = (gpu.config.width, gpu.config.height);
    let row = (w * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    buf
}

/// Writes a copied frame as a 24-bit BMP.
fn save_frame(gpu: &Gpu, buf: &wgpu::Buffer, path: &std::path::Path) {
    let (w, h) = (gpu.config.width, gpu.config.height);
    let row = (w * 4).div_ceil(256) * 256;
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(30)),
    });
    let Ok(data) = buf.slice(..).get_mapped_range() else {
        eprintln!("screenshot: mapping failed");
        return;
    };
    let bgra = matches!(
        gpu.config.format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let out_row = (w * 3).div_ceil(4) * 4;
    let mut bmp = Vec::with_capacity((54 + out_row * h) as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + out_row * h).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(w as i32).to_le_bytes());
    bmp.extend_from_slice(&(h as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let start = bmp.len();
        for x in 0..w {
            let i = (y * row + x * 4) as usize;
            let (r, g, b) = if bgra {
                (data[i + 2], data[i + 1], data[i])
            } else {
                (data[i], data[i + 1], data[i + 2])
            };
            bmp.extend_from_slice(&[b, g, r]);
        }
        bmp.resize(start + out_row as usize, 0);
    }
    match std::fs::write(path, bmp) {
        Ok(()) => println!("screenshot: {}", path.display()),
        Err(e) => eprintln!("screenshot: {e}"),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("error: cannot open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        match self.init_gpu(event_loop, Arc::clone(&window)) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                eprintln!("error: {e}");
                event_loop.exit();
                return;
            }
        }
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        ));
        self.window = Some(window);
        if let Some(name) = self.opts.autoplay.clone() {
            let root = self.opts.data_dir.join("worlds");
            let existing = client::worlds::list(&root)
                .into_iter()
                .find(|(_, i)| i.name == name);
            let dir = match existing {
                Some((d, _)) => Ok(d),
                None => client::worlds::create(&root, &name, client::worlds::seed_from_text("7")),
            };
            match dir {
                Ok(d) => self.open_world(d),
                Err(e) => self.menu.message = Some(e.to_string()),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(state), Some(window)) = (&mut self.egui_state, &self.window) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.game.is_some() {
                    self.close_world();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.depth = depth_view(&gpu.device, gpu.config.width, gpu.config.height);
                }
            }
            WindowEvent::Focused(false) => {
                if let Some(g) = &mut self.game
                    && !g.paused
                {
                    g.paused = true;
                    g.keys = Keys::default();
                    self.set_grab(false);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && self.game.is_some()
                    && !event.repeat
                {
                    self.game_key(code, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } if self.playing() => {
                if !self.grabbed {
                    self.set_grab(true);
                }
                if let Some(g) = &mut self.game {
                    match button {
                        MouseButton::Left => {
                            g.session.dig();
                        }
                        MouseButton::Right => {
                            if let Some(&(id, _)) = g.hotbar.get(g.selected) {
                                g.session.place(id);
                            }
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.playing() => {
                if let Some(g) = &mut self.game {
                    let up = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                        MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                    };
                    // Flying: the wheel sets the fly speed; walking: the hotbar.
                    if g.session.is_flying() {
                        g.session.adjust_fly_speed(if up { 1 } else { -1 });
                        return;
                    }
                    let n = g.hotbar.len().max(1);
                    g.selected = if up {
                        (g.selected + n - 1) % n
                    } else {
                        (g.selected + 1) % n
                    };
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.playing()
            && self.grabbed
        {
            let s = f64::from(self.settings.mouse_sensitivity) / 1000.0;
            if let Some(g) = &mut self.game {
                g.session.look(-delta.0 * s, -delta.1 * s);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(game) = self.game.take()
            && let Err(e) = game.session.close()
        {
            eprintln!("error: saving the world failed: {e}");
        }
    }
}
