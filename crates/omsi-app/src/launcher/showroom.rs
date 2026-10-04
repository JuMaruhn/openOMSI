//! The launcher's bus preview: the chosen bus drawn by the game's own renderer - its
//! model, paint and materials - on the plain (Vanilla+) path without the costly passes,
//! under the light of the chosen time and weather, into a picture the launcher shows
//! in a card. It is drawn again only when something changed (another bus, paint, light,
//! the preview turned by the mouse), never every frame.
//!
//! `Look::effects` picks between two pictures of the same bus: the card beside the bus list
//! takes the plain one, on one fixed path with no fog, wet or snow; the Vehicle Editor takes
//! the game's, in the renderer the settings ask for. The launcher's window is opened with
//! `showroom_options` either way, so its ambient occlusion stays off and its shadow map
//! small - those belong to the window, not to the light.
//!
//! A bus is read on a worker (its type, its scripts run to their resting state, its
//! textures and meshes put on the GPU ahead) and then placed into a scene of its own with
//! a fresh `World` (whose caches belong to one scene); the old scene goes when the new one
//! is ready, so the picture never goes blank while switching.

use super::super::*;
use glam::DVec3;
use omsi_render::{Camera, Lighting, Renderer, Scene};
use std::sync::mpsc::{channel, Receiver};

/// Where the camera stands. The Vehicle Editor switches between them with the keys the game
/// uses for its own views (F1, F2, F3; see `launcher::vehicle_editor`); the card beside the
/// bus list is always [`EditorCam::Outside`].
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum EditorCam {
    /// Turning about the bus: the picture this page has always shown.
    #[default]
    Outside,
    /// At one of the bus's own `[add_camera_driver]` eyes.
    Driver(usize),
    /// At one of its `[add_camera_pax]` eyes.
    Pax(usize),
    /// Standing anywhere, looking anywhere: the game's own F4, the one view that flies.
    Free,
}

/// A camera of the showroom: where it stands and where it is easing to. Each page keeps
/// its own (see [`Showroom`]).
#[derive(Clone)]
struct Cam {
    /// Orbit: yaw and pitch (degrees) and distance factor, eased towards the targets.
    yaw: f32,
    pitch: f32,
    zoom: f32,
    yaw_to: f32,
    pitch_to: f32,
    zoom_to: f32,
    /// The view it stands in (the Vehicle Editor's F1..F4; the card is always `Outside`).
    view: EditorCam,
    /// How far the head is turned from where the view's own eye looks, or - in `Free` - the
    /// direction it looks in outright (degrees).
    look: (f32, f32),
    /// `Free` only: where the camera stands in the world.
    pos: DVec3,
    idle: f32,
}

impl Cam {
    fn new() -> Cam {
        Cam {
            yaw: START_YAW,
            pitch: START_PITCH,
            zoom: 1.0,
            yaw_to: START_YAW,
            pitch_to: START_PITCH,
            zoom_to: 1.0,
            view: EditorCam::default(),
            look: (0.0, 0.0),
            pos: DVec3::ZERO,
            idle: 0.0,
        }
    }
}

/// Where the outside view stands before anyone turns it.
const START_YAW: f32 = 215.0;
const START_PITCH: f32 = 8.0;

/// What the showroom shows.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Look {
    pub root: PathBuf,
    pub map: String,
    pub bus: String,
    pub paint: String,
    pub weather: String,
    /// Minutes of the day, and the date.
    pub time: i32,
    pub date: String,
    /// Draw the weather as the game draws it: fog at the file's visibility, the wet sheen
    /// and the light snow throws back, and the bus's own scripts told what is falling (the
    /// drop film on its windows). The Drive page's card leaves this off and keeps the plain
    /// picture it has always had; the Vehicle Editor, which is there to look at exactly
    /// these things, asks for it.
    pub effects: bool,
    /// With `effects`: the renderer to draw in (`vanilla`, `vanilla_plus`, `enhanced`), the
    /// Vehicle Editor's own choice rather than the settings'. Empty without it, so changing
    /// it on that page never makes the Drive page's card a different picture.
    pub graphics: String,
    /// With `effects`: how far the wipers have swept their part of the panes clear (0..1,
    /// 1 = clean). It only ever stands the film where a sweep would have left it; the
    /// wipers themselves do not run in a picture that holds still.
    pub wiped: f32,
}

struct Ready {
    look: Look,
    world: Arc<scene::World>,
    vt: Arc<omsi_sim::VehicleType>,
    vehicle: omsi_sim::VehicleInstance,
    scheme: Option<usize>,
    mirrors: MirrorSetup,
}

/// How this bus's mirrors have been set in the cab: the same `mirrors.cfg` the game reads
/// when it spawns the bus (`settings::mirror_offsets` and the two beside it, keyed by the
/// vehicle file). A mirror aimed while driving is aimed the same way in the editor.
#[derive(Default, Clone)]
struct MirrorSetup {
    offsets: Vec<[f32; 2]>,
    shifts: Vec<[f32; 3]>,
    fovs: Vec<f32>,
}

struct Shown {
    look: Look,
    scene: Scene,
    #[allow(dead_code)]
    world: Option<Arc<scene::World>>,
    vehicle: Option<omsi_sim::VehicleInstance>,
    render: Option<scene::VehicleRender>,
    trailers: Vec<scene::VehicleRender>,
    /// Centre and size of the bus (its bounding box).
    centre: glam::Vec3,
    length: f32,
    weather: omsi_content::weather::Weather,
    lighting: Lighting,
    mirrors: MirrorSetup,
}

pub struct Showroom {
    shown: Option<Shown>,
    wanted: Option<Look>,
    /// The last look that could not be shown (not read again until something changes).
    failed: Option<Look>,
    loading: Option<(Look, Receiver<Result<Ready, String>>)>,
    pub error: Option<String>,
    /// One camera per page: the card beside the bus list and the Vehicle Editor keep their
    /// own, so turning the bus on one leaves the other where it was. They share the scene,
    /// not the view of it. `on_editor` says whose turn it is, from the `Look` last wanted.
    card: Cam,
    editor: Cam,
    on_editor: bool,
    pub auto_turn: bool,
    /// Where the bus should appear on screen: the share of the width its centre is at.
    pub focus_x: f32,
    focus_now: f32,
    pub busy: bool,
    /// How wet the unwiped panes settled on this weather (see [`Showroom::wetness`]).
    wetness: f32,
    /// Where the picture last stood, so the free view can start where it is looking.
    last_cam: Option<(DVec3, f32, f32)>,
    /// The picture: its texture and size, and whether it must be drawn again.
    target: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    dirty: bool,
    /// Bumped whenever `target` was made anew (the interface binds it again).
    pub generation: u64,
}

fn args_for(look: &Look) -> Args {
    let mut v = vec!["omsi".to_string(), "--root".into(), look.root.to_string_lossy().to_string()];
    if !look.map.is_empty() {
        v.extend(["--map".into(), look.map.clone()]);
    }
    if !look.bus.is_empty() {
        v.extend(["--bus".into(), look.bus.clone()]);
    }
    if !look.paint.is_empty() {
        v.extend(["--paint".into(), look.paint.clone()]);
    }
    // (the current weather is fetched by the game, not by the preview)
    if !look.weather.is_empty() && !look.weather.starts_with("metar:") {
        v.extend(["--weather".into(), look.weather.clone()]);
    }
    v.extend(["--time".into(), format!("{:02}:{:02}", look.time / 60, look.time % 60)]);
    if !look.date.is_empty() {
        v.extend(["--date".into(), look.date.clone()]);
    }
    Args::try_parse_from(v).unwrap_or_else(|_| Args::parse_from(["omsi"]))
}

impl Showroom {
    pub fn new() -> Showroom {
        Showroom {
            shown: None,
            wanted: None,
            failed: None,
            loading: None,
            error: None,
            card: Cam::new(),
            editor: Cam::new(),
            on_editor: false,
            auto_turn: false,
            focus_x: 0.5,
            focus_now: 0.5,
            busy: false,
            wetness: 0.0,
            last_cam: None,
            target: None,
            dirty: true,
            generation: 0,
        }
    }

    /// Show this (a bus, its paint, the time and weather to light it with).
    pub fn want(&mut self, look: Look) {
        // whose camera the next drag, key and frame belong to (see [`Showroom`])
        self.on_editor = look.effects;
        if self.wanted.as_ref() != Some(&look) {
            self.wanted = Some(look);
        }
    }

    /// The camera of the page being drawn (see [`Showroom`]).
    fn cam(&self) -> &Cam {
        if self.on_editor { &self.editor } else { &self.card }
    }
    fn cam_mut(&mut self) -> &mut Cam {
        if self.on_editor { &mut self.editor } else { &mut self.card }
    }

    /// Which view the page stands in, and whether it is the free one.
    pub fn view(&self) -> EditorCam {
        self.cam().view
    }

    /// Take a view (F1..F4). `Free` starts where the picture is looking now, as the game's
    /// own free camera does when F4 is pressed; the others look where their eye looks.
    pub fn set_view(&mut self, view: EditorCam) {
        let last = self.last_cam;
        let c = self.cam_mut();
        c.view = view;
        c.look = (0.0, 0.0);
        // the zoom is a distance outside and a field of view at an eye, so it does not
        // carry from one to the other: a view just taken is the view as it is meant
        (c.zoom, c.zoom_to) = (1.0, 1.0);
        if view == EditorCam::Free {
            if let Some((p, yaw, pitch)) = last {
                (c.pos, c.look) = (p, (yaw, pitch));
            }
        }
        self.dirty = true;
    }

    /// Step to another of the eyes the bus offers of the kind in use.
    pub fn step_view(&mut self, view: EditorCam) {
        let c = self.cam_mut();
        c.view = view;
        c.look = (0.0, 0.0);
        self.dirty = true;
    }

    /// The mouse dragged over the empty part of the window (degrees), or turned the wheel.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        let c = self.cam_mut();
        c.yaw_to += dx * 0.35;
        c.pitch_to = (c.pitch_to + dy * 0.25).clamp(-4.0, 55.0);
        c.idle = 0.0;
    }

    /// Turn the outside view by so many degrees. `orbit` takes the mouse's pixels and puts
    /// its own gain on them; the arrow keys already know how far they mean to turn.
    pub fn turn(&mut self, dx: f32, dy: f32) {
        let c = self.cam_mut();
        c.yaw_to += dx;
        c.pitch_to = (c.pitch_to + dy).clamp(-4.0, 55.0);
        c.idle = 0.0;
    }

    /// Everything looking where it looked at the start, and the free camera back with it
    /// (`view_reset_all_directions`).
    pub fn reset_turn(&mut self) {
        let c = self.cam_mut();
        (c.yaw_to, c.pitch_to, c.zoom_to) = (START_YAW, START_PITCH, 1.0);
        c.look = (0.0, 0.0);
        c.pos = DVec3::ZERO;
        c.view = EditorCam::Outside;
        c.idle = 0.0;
        self.dirty = true;
    }

    /// The picture shown is the Vehicle Editor's, the only page with a view but `Outside`.
    fn editor(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.look.effects)
    }

    /// Fly the free camera `metres` along its own axes - forward, to the right, upwards -
    /// as the game flies its own with W/A/S/D/Q/E (`app_events`). Only `Free` flies: the
    /// other views stand where the bus puts them, which is what they are for.
    pub fn fly(&mut self, fwd: f32, right: f32, up: f32, metres: f32) {
        let c = self.cam_mut();
        if c.view != EditorCam::Free {
            return;
        }
        let (yaw, pitch) = c.look;
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        let f = DVec3::new((sy * cp) as f64, (cy * cp) as f64, sp as f64);
        // level with the ground, to the right of the view (x east, y north, z up)
        let r = DVec3::new(cy as f64, -sy as f64, 0.0);
        let v = f * fwd as f64 + r * right as f64 + DVec3::Z * up as f64;
        if v.length_squared() > 1e-9 {
            c.pos += v.normalize() * metres as f64;
            c.idle = 0.0;
            self.dirty = true;
        }
    }

    /// Turn the view it is in by so many degrees (the arrow keys, which say how far they
    /// mean to turn; `drag` takes the mouse's pixels and puts its own gain on them).
    pub fn drag_by_degrees(&mut self, dx: f32, dy: f32) {
        if drag_orbits(self.editor(), self.cam().view) {
            self.turn(dx, dy);
            return;
        }
        let c = self.cam_mut();
        c.look.0 += dx;
        c.look.1 = (c.look.1 - dy).clamp(-85.0, 85.0);
        c.idle = 0.0;
        self.dirty = true;
    }

    /// The mouse dragged over the picture: it turns the view it is in - round the bus
    /// outside, the head or the free camera inside it, as the game turns a head in the cab.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        if drag_orbits(self.editor(), self.cam().view) {
            self.orbit(dx, dy);
            return;
        }
        let c = self.cam_mut();
        c.look.0 += dx * 0.35;
        c.look.1 = (c.look.1 - dy * 0.25).clamp(-85.0, 85.0);
        c.idle = 0.0;
        self.dirty = true;
    }

    /// Draw the picture again: the camera moved, which no `Look` says.
    pub fn redraw(&mut self) {
        self.dirty = true;
    }

    /// The wheel. Outside it is the distance the camera stands at, as it has always been;
    /// at an eye of the bus and in the free view there is no distance to change, so it is
    /// the field of view instead, the way Omsi.exe zooms a view of the bus
    /// (`app_events`: `fov_deg = base * view_zoom`, 8 to 120 degrees). The range is wider
    /// there for that reason: 0.16 of a 50-degree eye is the 8 degrees the original allows.
    pub fn zoom_by(&mut self, k: f32) {
        let c = self.cam_mut();
        let (lo, hi) = if c.view == EditorCam::Outside { (0.55, 2.2) } else { (0.16, 2.4) };
        c.zoom_to = (c.zoom_to * k).clamp(lo, hi);
        c.idle = 0.0;
    }

    /// Per frame: start loading what is wanted, take over what finished loading, move the
    /// camera. Returns true when a new scene was put in place.
    pub fn update(&mut self, renderer: &Renderer, dt: f32) -> bool {
        let mut swapped = false;
        // what finished loading
        if let Some((look, rx)) = self.loading.as_ref() {
            match rx.try_recv() {
                Ok(Ok(ready)) => {
                    self.loading = None;
                    self.shown = Some(self.place(renderer, ready));
                    self.error = None;
                    self.dirty = true;
                    swapped = true;
                }
                Ok(Err(e)) => {
                    log::warn!("showroom {}: {e}", look.bus);
                    self.error = Some(e);
                    // (the bus before stayed in the picture as if it were the one chosen, and
                    // the failed one was read again every frame)
                    self.failed = Some(look.clone());
                    self.shown = None;
                    self.loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => {
                    self.failed = Some(look.clone());
                    self.loading = None;
                }
            }
        }
        // what is wanted and not shown or loading
        if let Some(w) = self.wanted.clone() {
            let shown = self.shown.as_ref().map(|s| &s.look);
            let loading = self.loading.as_ref().map(|l| &l.0);
            if shown != Some(&w) && loading != Some(&w) && self.loading.is_none() && self.failed.as_ref() != Some(&w) {
                // only the light changed: no need to read the bus again
                let same_bus = shown.map(|s| s.bus == w.bus && s.paint == w.paint && s.map == w.map && s.root == w.root).unwrap_or(false);
                if same_bus {
                    if let Some(s) = self.shown.as_mut() {
                        s.look = w.clone();
                        let args = args_for(&w);
                        s.weather = load_weather(&args);
                        setup_sky(&args, renderer, &mut s.scene, omsi_content::Envir::load(&args.root.join("envir.cfg")).ok().as_ref(), Some(&s.weather));
                        s.lighting = lighting_for(&args, &s.weather, w.effects, &w.graphics);
                        if w.effects {
                            if let Some(v) = s.vehicle.as_mut() {
                                self.wetness = weather_to_scripts(v, &s.weather, w.wiped);
                            }
                        }
                    }
                    self.dirty = true;
                } else {
                    self.start_loading(renderer, w);
                }
            }
        }
        self.busy = self.loading.is_some();
        // camera
        let auto = self.auto_turn;
        let c = self.cam_mut();
        c.idle += dt;
        if auto && c.idle > 4.0 {
            c.yaw_to += dt * 6.0;
        }
        let k = 1.0 - (-dt / 0.18).exp();
        // still turning: the picture must follow
        let moving = (c.yaw_to - c.yaw).abs() > 0.05 || (c.pitch_to - c.pitch).abs() > 0.05 || (c.zoom_to - c.zoom).abs() > 0.001;
        c.yaw += (c.yaw_to - c.yaw) * k;
        c.pitch += (c.pitch_to - c.pitch) * k;
        c.zoom += (c.zoom_to - c.zoom) * k;
        if moving {
            self.dirty = true;
        }
        self.focus_now += (self.focus_x - self.focus_now) * (1.0 - (-dt / 0.35).exp());
        // the bus's own state, whenever the picture is drawn again
        if let (true, Some(s)) = (self.dirty, self.shown.as_mut()) {
            if let (Some(v), Some(r)) = (s.vehicle.as_mut(), s.render.as_mut()) {
                player::sync_vehicle_transforms(renderer, &mut s.scene, v, r, &mut s.trailers, false);
            }
        }
        swapped
    }

    fn start_loading(&mut self, renderer: &Renderer, look: Look) {
        let args = args_for(&look);
        let root = look.root.clone();
        let map_cfg = omsi_cfg::resolve_path(&root, &look.map);
        let date = start_clock(&args).date_code();
        let t0 = std::time::Instant::now();
        let world = match scene::World::open(&root, &map_cfg, date) {
            Ok(w) => {
                log::info!("showroom: {} opened in {:.2} s", map_cfg.display(), t0.elapsed().as_secs_f64());
                Arc::new(w)
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                self.wanted = None;
                return;
            }
        };
        let prefetch = world.vehicle_prefetch(renderer);
        let (tx, rx) = channel();
        let l2 = look.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<Ready> {
                if l2.bus.is_empty() {
                    return Err(anyhow!("no bus"));
                }
                let path = player_bus_path(&root, &l2.bus)?;
                let vt = Arc::new(omsi_sim::VehicleType::load(&root, &path)?);
                let scheme = paint_scheme(&vt, Some(l2.paint.as_str()).filter(|p| !p.is_empty()));
                let host = omsi_sim::VehicleHost::new(start_clock(&args));
                let mut vehicle = omsi_sim::VehicleInstance::new(vt.clone(), host);
                if let Some(i) = scheme {
                    for (var, v) in vt.paint_schemes[i].set_vars.clone() {
                        vehicle.set_var(&var, v);
                    }
                }
                load_coupled_parts(&root, &mut vehicle);
                for _ in 0..3 {
                    vehicle.update(1.0 / 30.0);
                }
                prefetch.prefetch(&vt, scheme);
                for t in &vehicle.trailers {
                    prefetch.prefetch(&t.ty, scheme.filter(|i| *i < t.ty.paint_schemes.len()));
                }
                // how the mirrors have been aimed in the cab (see [`MirrorSetup`])
                let mirrors = MirrorSetup {
                    offsets: crate::settings::mirror_offsets(&vt.def.path),
                    shifts: crate::settings::mirror_shifts(&vt.def.path),
                    fovs: crate::settings::mirror_fovs(&vt.def.path),
                };
                Ok(Ready { look: l2, world, vt, vehicle, scheme, mirrors })
            })();
            let _ = tx.send(r.map_err(|e| format!("{e:#}")));
        });
        self.loading = Some((look, rx));
    }

    fn place(&mut self, renderer: &Renderer, r: Ready) -> Shown {
        let t0 = std::time::Instant::now();
        let mut scene = renderer.new_scene();
        let args = args_for(&r.look);
        let weather = load_weather(&args);
        let envir = omsi_content::Envir::load(&args.root.join("envir.cfg")).ok();
        setup_sky(&args, renderer, &mut scene, envir.as_ref(), Some(&weather));
        add_floor(renderer, &mut scene);
        let world = r.world;
        let mut vehicle = r.vehicle;
        vehicle.position = DVec3::ZERO;
        vehicle.heading = 0.0;
        let render = world.add_vehicle(renderer, &mut scene, &r.vt, r.scheme);
        let trailers: Vec<scene::VehicleRender> = vehicle.trailers.iter().map(|t| world.add_vehicle_part(renderer, &mut scene, &t.ty, r.scheme.filter(|i| *i < t.ty.paint_schemes.len()), &render)).collect();
        vehicle.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
        for t in vehicle.trailers.iter_mut() {
            t.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
        }
        if r.look.effects {
            self.wetness = weather_to_scripts(&mut vehicle, &weather, r.look.wiped);
        }
        vehicle.update(1.0 / 30.0);
        // the bus's size from its bounding box (with the rear section behind it)
        let bb = r.vt.def.bounding_box.unwrap_or([2.5, 12.0, 3.0, 0.0, 0.0, 1.5]);
        let mut length = bb[1];
        let mut centre = glam::Vec3::new(bb[3], bb[4], bb[5]);
        for t in &vehicle.trailers {
            let tb = t.ty.def.bounding_box.unwrap_or([2.5, 8.0, 3.0, 0.0, 0.0, 1.5]);
            let back = (t.position - vehicle.position).truncate().length() as f32 + tb[1] * 0.5;
            let total = bb[1] * 0.5 + back;
            centre.y = bb[4] + bb[1] * 0.5 - total * 0.5;
            length = total;
        }
        let lighting = lighting_for(&args, &weather, r.look.effects, &r.look.graphics);
        log::info!("showroom: {} ({} meshes, {:.1} m long) placed in {:.2} s", r.look.bus, render.instances.len(), length, t0.elapsed().as_secs_f64());
        Shown { look: r.look, scene, world: Some(world), vehicle: Some(vehicle), render: Some(render), trailers, centre, length, weather, lighting, mirrors: r.mirrors }
    }

    /// The picture of the bus at `w` x `h` pixels, drawn again when something changed.
    pub fn preview(&mut self, renderer: &mut Renderer, w: u32, h: u32) -> Option<wgpu::TextureView> {
        self.shown.as_ref()?;
        let (w, h) = (w.max(16), h.max(16));
        if self.target.as_ref().map(|t| (t.2, t.3) != (w, h)).unwrap_or(true) {
            let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bus preview"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.target = Some((tex, view, w, h));
            self.generation += 1;
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            let view = self.target.as_ref().unwrap().1.clone();
            self.render(renderer, &view, w, h);
        }
        self.target.as_ref().map(|t| t.1.clone())
    }

    fn render(&mut self, renderer: &mut Renderer, target: &wgpu::TextureView, w: u32, h: u32) {
        let (c, focus) = (self.cam().clone(), self.focus_now);
        let (yaw, pitch, zoom) = (c.yaw, c.pitch, c.zoom);
        let Some(s) = self.shown.as_mut() else { return };
        let fov = 30.0f32;
        let aspect = w as f32 / h.max(1) as f32;
        // far enough that the whole bus fits the free part of the window
        let half_v = (fov.to_radians() * 0.5).tan();
        // (the free part is the share of the window right of the panels: twice the room
        // from the focus to the right edge)
        let free = ((1.0 - focus) * 2.0).clamp(0.3, 1.0);
        let fit = (s.length * 0.5 + 1.0) / (half_v * aspect * free * 0.92).max(0.1);
        let dist = (fit * zoom).max(8.0);
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        let target_pt = DVec3::new(s.centre.x as f64, s.centre.y as f64, (s.centre.z * 0.75) as f64);
        // from the camera towards the bus: forward along the yaw, down by the pitch
        let dir = DVec3::new((sy * cp) as f64, (cy * cp) as f64, -sp as f64);
        let mut pos = target_pt - dir * dist as f64;
        pos.z = pos.z.max(0.6);
        // the bus stands in the free part of the window: the camera looks past it to the
        // side by as much
        let side = (focus - 0.5) * 2.0 * half_v * aspect;
        let look_yaw = yaw - side.atan().to_degrees();
        let mut cam = Camera { position: pos, yaw: look_yaw, pitch: -pitch, roll: 0.0, fov_deg: fov, near: 0.2, far: 6000.0 };
        // Inside the bus (the Vehicle Editor's F1 and F2): its own eye, turned by the look.
        // Only there - the card beside the bus list keeps the turning view it has always
        // had, whatever this is left at.
        if s.look.effects {
            match c.view {
                // standing anywhere, looking anywhere (F4)
                EditorCam::Free => {
                    cam = Camera { position: c.pos, yaw: c.look.0, pitch: c.look.1, roll: 0.0, fov_deg: zoomed(55.0, zoom), near: 0.05, far: 6000.0 };
                }
                // at one of the bus's own eyes (F1, F2), turned by the look
                view => {
                    if let (Some(e), Some(v)) = (interior_camera(s, view), s.vehicle.as_ref()) {
                        let (eye, cyaw, cpitch) = v.camera_world(&e);
                        // the eye's own field of view, zoomed as Omsi.exe zooms one
                        let fov = zoomed(if e.fov > 1.0 { e.fov } else { 50.0 }, zoom);
                        cam = Camera { position: eye, yaw: cyaw + c.look.0, pitch: cpitch + c.look.1, roll: 0.0, fov_deg: fov, near: 0.05, far: 6000.0 };
                    }
                }
            }
        }
        self.last_cam = Some((cam.position, cam.yaw, cam.pitch));
        let Some(s) = self.shown.as_mut() else { return };
        s.scene.overlays.clear();
        let _ = &s.weather;
        // A picture on its own. The enhanced path's exposure and sky follow the light over a
        // second or two of frames, and the showroom draws one frame per change - those
        // frames never come, so the bus stood in a grey, flat picture at whatever exposure
        // the renderer started with. `instant_exposure` puts both where the light wants them
        // at once, as `Renderer::render_to_image` does for an offscreen shot. Only for the
        // page that draws enhanced at all: the card beside the bus list keeps its frame, and
        // the fuller reflection probe this also asks for costs it nothing.
        let standalone = s.look.effects;
        if standalone {
            renderer.instant_exposure = true;
            // the bus's own mirrors first: the picture samples what they hold (see [`mirrors`])
            mirrors(renderer, s, &cam, aspect);
        }
        renderer.render(&mut s.scene, target, w, h, &cam, &s.lighting);
        if standalone {
            renderer.instant_exposure = false;
        }
    }

    /// How wet the unwiped panes stand (0..1), as the scripts settled them on this weather:
    /// what the Vehicle Editor's wiper slider is a share of.
    pub fn wetness(&self) -> f32 {
        self.wetness
    }

    /// A bus is there to show.
    pub fn has_picture(&self) -> bool {
        self.shown.as_ref().map(|s| s.vehicle.is_some()).unwrap_or(false) && self.target.is_some()
    }

}

/// The bus's mirrors into their own textures (`reflexionN.bmp`), the way the game draws
/// them: `camera_util::render_mirrors` is the game's pass, given the showroom's vehicle and
/// scene instead of a player's (see `render_vehicle_mirrors`). Each mirror is aimed from the
/// eye of the picture being drawn, as Omsi.exe aims them at whoever looks into the glass, so
/// turning round the bus turns what its mirrors show; the glass itself is already a render
/// texture in every scene (`scene::World::mirror_texture`), it was only never drawn into
/// here, and a mirror then stood empty.
///
/// Only the Vehicle Editor pays for this. The card beside the bus list is a thumbnail of the
/// bus's own body and keeps its one pass. The mirrors out of the picture are not redrawn, as
/// the game leaves them (`mirror_in_view`), and `Mirrors: off` in the settings - the size 0
/// that switches them off in the game - leaves them alone here too.
///
/// All of them are drawn at once, where the game takes turns among them over its frames: the
/// showroom draws a picture only when something changed, so a bus standing still costs
/// nothing, and a mirror a few frames behind the view being dragged would be the one thing
/// the page is there to look at. (A seven-mirror bus: about 4 ms beside the 2 ms the picture
/// itself takes, at the 256-pixel default.)
fn mirrors(renderer: &mut Renderer, s: &mut Shown, cam: &Camera, aspect: f32) {
    if crate::MIRROR_SIZE.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    // (field by field: the pass wants the scene by itself while it reads the bus beside it)
    let Shown { scene, vehicle, world, lighting, mirrors, .. } = s;
    let (Some(v), Some(w)) = (vehicle.as_ref(), world.as_ref()) else { return };
    if v.ty.def.cameras_reflexion.is_empty() {
        return;
    }
    let aim = crate::camera_util::MirrorAim { offsets: &mirrors.offsets, shifts: &mirrors.shifts, fovs: &mirrors.fovs };
    crate::camera_util::render_vehicle_mirrors(renderer, scene, w, v, aim, cam.position, lighting, None, Some((*cam, aspect)));
}

/// The light of the look's time and weather, with the sun's shadow under the bus. Always
/// the plain renderer, whatever the game's graphics setting: a preview is to be quick and
/// clear, not the game's picture (no enhanced exposure and glow, no weather effects).
/// The light of the chosen time and weather. `weather_lighting` is the game's own, so what
/// comes back is already right - including the renderer the settings ask for, which
/// `launcher_statics` put within its reach when the launcher started.
///
/// Without `effects` six of its answers are taken back: the card beside the bus list is a
/// thumbnail drawn on one fixed path, and a thumbnail of a bus in 75 m of ground fog would
/// be a grey square. With them the picture is the game's.
fn lighting_for(args: &Args, weather: &omsi_content::weather::Weather, effects: bool, graphics: &str) -> Lighting {
    let clock = start_clock(args);
    let envir = omsi_content::Envir::load(&args.root.join("envir.cfg")).ok();
    let daylight = omsi_sim::Daylight::compute(&clock, envir.as_ref());
    let wetness = if effects { crate::weather_setup::initial_wetness(weather) } else { 0.0 };
    let mut l = weather_lighting(&daylight, weather, crate::weather_setup::cloud_drift_at(weather, clock.time), wetness, true);
    l.shadows = daylight.altitude_deg > 2.0;
    if effects {
        // the renderer the page is set to, over the settings' own that `weather_lighting`
        // took from the statics; the detail grain is not one of those, so it comes from the
        // settings here as it does in the game (`app_events.rs`). The file is read on a
        // relight, not on a frame.
        l.enhanced = graphics == "enhanced";
        l.classic = graphics == "vanilla";
        l.detail = crate::settings::Settings::load().detail_textures;
    } else {
        l.enhanced = false;
        l.classic = false;
        l.detail = false;
        l.wetness = 0.0;
        l.snow = 0.0;
        l.fog_density = 0.0;
    }
    l
}

/// The variables that carry a film of rain on this bus's panes: the `[matl_alphascale]` of
/// a material whose name begins `rain_window`, which is how both the renderer and the
/// vehicle know one (`scene.rs`, `omsi_sim::vehicle`, and `lan.rs` to send them).
fn rain_films(ty: &omsi_sim::VehicleType) -> Vec<String> {
    let mut v: Vec<String> = ty
        .model
        .meshes
        .iter()
        .flat_map(|m| m.materials.iter())
        .filter_map(|mat| mat.alphascale.clone())
        .filter(|n| n.trim().to_ascii_lowercase().starts_with("rain_window"))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Tell the bus's scripts what the weather is doing, as the game tells them
/// (`weather_setup::apply_weather`): the precipitation type and rate, the state of the road
/// and the air. Then let them run until the picture has settled on it.
///
/// The film on the panes is no texture the weather switches on: it is a value the bus's own
/// script builds up. The stock `rain.osc` is
///
/// ```text
/// (L.S.Timegap) (L.L.PrecipRate) * s0
/// (L.L.Rain_Window_Norm_Wetness) l0 + 1 min 0 max (S.L.Rain_Window_Norm_Wetness)
/// ```
///
/// - the wetness gains with every frame of rain until it reaches 1, and
/// `[matl_alphascale] Rain_Window_Norm_Wetness` then shows the drops (with the vehicle's 1.8
/// boost, `omsi_sim::vehicle`). The showroom draws one frame per change, so a handful of
/// ticks left every pane as good as dry.
///
/// How long that takes is the script's business, not ours, so the scripts are run until the
/// film stops moving rather than for a time worked out from the rate - a bus whose script
/// wets its panes by another rule settles just the same. `SETTLE_MAX` is only a stop for one
/// that never settles (a wiper sweeping, say).
///
/// They are also set back to dry first. That script only ever adds: at `PrecipRate` 0
/// nothing takes the water off again, so a bus looked at in the rain stayed wet under a
/// clear sky, and every further rainy weather went on from where the last had stopped. This
/// page shows one weather, not the ones looked at before it.
fn weather_to_scripts(vehicle: &mut omsi_sim::VehicleInstance, weather: &omsi_content::weather::Weather, wiped: f32) -> f32 {
    const STEP: f32 = 1.0 / 30.0;
    const SETTLE_MAX: f32 = 20.0;
    let films = rain_films(&vehicle.ty.clone());
    for n in &films {
        vehicle.set_var(n, 0.0);
    }
    crate::weather_setup::apply_weather(vehicle, weather, crate::weather_setup::initial_wetness(weather));
    let wettest = |v: &omsi_sim::VehicleInstance| films.iter().filter_map(|n| v.var(n)).fold(0.0f32, f32::max);
    let mut last = wettest(vehicle);
    for step in 0..(SETTLE_MAX / STEP) as u32 {
        vehicle.update(STEP);
        let now = wettest(vehicle);
        // (a few frames for the rest of the bus whatever the panes do)
        if step >= 2 && (now - last).abs() < 1e-4 {
            break;
        }
        last = now;
    }
    // The wipers do not run in a picture that holds still, so their part of the glass is put
    // where a sweep would have left it: `wiped` of the way from as wet as the rest to dry.
    // `update(0.0)` then has the meshes read the new value without `rain.osc` putting a
    // frame of rain back on (it gains `Timegap * PrecipRate`, and Timegap is 0).
    for n in films.iter().filter(|n| n.to_ascii_lowercase().contains("wiped")) {
        vehicle.set_var(n, last * (1.0 - wiped).clamp(0.0, 1.0));
    }
    vehicle.update(0.0);
    last
}

/// A field of view zoomed by `k`, held within the 8 to 120 degrees Omsi.exe holds one in
/// (`app_events`). Below one it is zoomed in, as a smaller `zoom` stands the outside camera
/// closer.
fn zoomed(base: f32, k: f32) -> f32 {
    (base * k).clamp(8.0, 120.0)
}

/// Whether a drag turns the picture round the bus rather than turning a head inside it.
/// Outside it always does, and so does every page but the Vehicle Editor: the card beside
/// the bus list has no inside view to turn a head in.
fn drag_orbits(editor: bool, view: EditorCam) -> bool {
    !editor || view == EditorCam::Outside
}

/// The bus's own camera a view names, taken as the game takes it: from `[camera_std]`
/// onwards, wrapping (`camera_util::driver_eye`). None outside, or where the bus gives none.
fn interior_camera(s: &Shown, view: EditorCam) -> Option<omsi_vehicle::Camera> {
    let def = &s.vehicle.as_ref()?.ty.def;
    let (list, i) = match view {
        EditorCam::Outside | EditorCam::Free => return None,
        EditorCam::Driver(i) => (&def.cameras_driver, i),
        EditorCam::Pax(i) => (&def.cameras_pax, i),
    };
    (!list.is_empty()).then(|| list[(def.camera_std + i) % list.len()].clone())
}

/// How many eyes a view has to step through (`view_interiorcam_plus/minus`).
impl Showroom {
    pub fn interior_count(&self, view: EditorCam) -> usize {
        let Some(s) = self.shown.as_ref() else { return 0 };
        let Some(def) = s.vehicle.as_ref().map(|v| &v.ty.def) else { return 0 };
        match view {
            EditorCam::Outside | EditorCam::Free => 0,
            EditorCam::Driver(_) => def.cameras_driver.len(),
            EditorCam::Pax(_) => def.cameras_pax.len(),
        }
    }
}

/// A round showroom floor under the bus: dark, matt, catching its shadow.
fn add_floor(renderer: &Renderer, scene: &mut Scene) {
    let n = 96;
    let r = 400.0f32;
    let mut positions = vec![glam::Vec3::ZERO];
    let mut normals = vec![glam::Vec3::Z];
    let mut uvs = vec![glam::Vec2::ZERO];
    for k in 0..n {
        let a = std::f32::consts::TAU * k as f32 / n as f32;
        positions.push(glam::Vec3::new(a.cos() * r, a.sin() * r, 0.0));
        normals.push(glam::Vec3::Z);
        uvs.push(glam::Vec2::new(a.cos(), a.sin()));
    }
    let mut indices = Vec::new();
    for k in 0..n {
        indices.extend([0u32, 1 + ((k + 1) % n) as u32, 1 + k as u32]);
    }
    let data = omsi_geometry::MeshData { positions, normals, uvs, ranges: vec![(0, indices.len() as u32, 0)], indices, one_sided: false };
    let mesh = renderer.add_mesh(scene, &data);
    let mat = renderer.add_material(scene, None, omsi_render::AlphaMode::Opaque, [0.12, 0.125, 0.135, 1.0], false);
    renderer.add_instance(scene, mesh, DVec3::new(0.0, 0.0, -0.005), glam::Mat4::IDENTITY, vec![mat]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The weather a file asks for reaches the picture only where the page wants it: the
    /// Vehicle Editor is there to look at fog, rain and snow, the Drive page's card is a
    /// thumbnail and keeps the plain light it has always had.
    #[test]
    fn only_a_page_that_asks_for_them_gets_fog_wet_and_snow() {
        let args = crate::cli::Args::parse_from(["openomsi"]);
        // heavy snow at 120 m: falling, lying, and the air thick with it
        let mut w = omsi_content::weather::Weather { snow: true, ..Default::default() };
        w.fog.0 = 120.0;
        w.precip = vec![2.0, 200.0];
        w.ground_wet[0] = 180.0;

        let plain = lighting_for(&args, &w, false, "");
        assert_eq!((plain.fog_density, plain.wetness, plain.snow), (0.0, 0.0, 0.0), "the card draws none of them");

        let full = lighting_for(&args, &w, true, "vanilla_plus");
        assert!(full.fog_density > 0.0, "120 m of visibility is fog");
        assert!(full.wetness > 0.0, "a wet road is wet");
        assert!(full.snow > 0.0, "snow lies");
        // the rest is the same light either way: only those three are held back
        assert_eq!(plain.sun_intensity, full.sun_intensity);
        assert_eq!(plain.shadows, full.shadows);
    }

    /// Each page keeps its own camera: turning the bus in the Vehicle Editor must not move
    /// the card beside the bus list, which shares the scene but not the view of it.
    #[test]
    fn the_two_pages_cameras_are_apart() {
        let mut sr = Showroom::new();
        sr.on_editor = true;
        sr.orbit(120.0, 0.0);
        sr.set_view(EditorCam::Driver(0));
        sr.on_editor = false;
        assert_eq!(sr.cam().yaw_to, START_YAW, "the card did not turn with it");
        assert_eq!(sr.cam().view, EditorCam::Outside, "nor did it take the editor's view");
        sr.on_editor = true;
        assert!(sr.cam().yaw_to > START_YAW, "and the editor kept its own");
        assert_eq!(sr.cam().view, EditorCam::Driver(0));
    }

    /// The wheel zooms an eye's field of view within the range the original holds one in.
    #[test]
    fn an_eyes_zoom_stays_within_the_originals_range() {
        assert_eq!(zoomed(50.0, 1.0), 50.0, "untouched, the eye's own");
        assert_eq!(zoomed(50.0, 0.16), 8.0, "the far end of the wheel is the original's 8 degrees");
        assert_eq!(zoomed(50.0, 0.05), 8.0, "and no further");
        assert_eq!(zoomed(60.0, 2.4), 120.0, "nor wider than 120");
    }

    /// A drag turns the head inside the bus and the picture round it outside - but only on
    /// the page that has the inside views at all.
    #[test]
    fn a_drag_turns_the_head_inside_and_the_picture_outside() {
        assert!(drag_orbits(true, EditorCam::Outside), "outside it turns the picture");
        assert!(!drag_orbits(true, EditorCam::Driver(0)), "at the wheel it turns the head");
        assert!(!drag_orbits(true, EditorCam::Pax(1)), "and in the saloon");
        assert!(!drag_orbits(true, EditorCam::Free), "and the free camera turns on the spot");
        assert!(drag_orbits(false, EditorCam::Driver(0)), "the card always turns the picture");
    }

    /// The editor draws in the renderer its own button is set to, each of the three a
    /// different pair of flags; the card draws on one fixed path whatever is chosen there.
    #[test]
    fn the_editors_button_picks_the_renderer_and_the_card_ignores_it() {
        let args = crate::cli::Args::parse_from(["openomsi"]);
        let w = omsi_content::weather::Weather::default();
        for (mode, enhanced, classic) in [("vanilla", false, true), ("vanilla_plus", false, false), ("enhanced", true, false)] {
            let l = lighting_for(&args, &w, true, mode);
            assert_eq!((l.enhanced, l.classic), (enhanced, classic), "{mode}");
            let card = lighting_for(&args, &w, false, mode);
            assert_eq!((card.enhanced, card.classic), (false, false), "the card stays plain under {mode}");
        }
    }
}
