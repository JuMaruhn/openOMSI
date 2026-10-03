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
}

struct Ready {
    look: Look,
    world: Arc<scene::World>,
    vt: Arc<omsi_sim::VehicleType>,
    vehicle: omsi_sim::VehicleInstance,
    scheme: Option<usize>,
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
}

pub struct Showroom {
    shown: Option<Shown>,
    wanted: Option<Look>,
    /// The last look that could not be shown (not read again until something changes).
    failed: Option<Look>,
    loading: Option<(Look, Receiver<Result<Ready, String>>)>,
    pub error: Option<String>,
    /// Orbit: yaw and pitch (degrees) and distance factor, eased towards the targets.
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    yaw_to: f32,
    pitch_to: f32,
    zoom_to: f32,
    pub auto_turn: bool,
    idle: f32,
    /// Where the bus should appear on screen: the share of the width its centre is at.
    pub focus_x: f32,
    focus_now: f32,
    pub busy: bool,
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
            yaw: 215.0,
            pitch: 8.0,
            zoom: 1.0,
            yaw_to: 215.0,
            pitch_to: 8.0,
            zoom_to: 1.0,
            auto_turn: false,
            idle: 0.0,
            focus_x: 0.5,
            focus_now: 0.5,
            busy: false,
            target: None,
            dirty: true,
            generation: 0,
        }
    }

    /// Show this (a bus, its paint, the time and weather to light it with).
    pub fn want(&mut self, look: Look) {
        if self.wanted.as_ref() != Some(&look) {
            self.wanted = Some(look);
        }
    }

    /// The mouse dragged over the empty part of the window (degrees), or turned the wheel.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw_to += dx * 0.35;
        self.pitch_to = (self.pitch_to + dy * 0.25).clamp(-4.0, 55.0);
        self.idle = 0.0;
    }
    pub fn zoom_by(&mut self, k: f32) {
        self.zoom_to = (self.zoom_to * k).clamp(0.55, 2.2);
        self.idle = 0.0;
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
                                weather_to_scripts(v, &s.weather);
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
        self.idle += dt;
        if self.auto_turn && self.idle > 4.0 {
            self.yaw_to += dt * 6.0;
        }
        let k = 1.0 - (-dt / 0.18).exp();
        // still turning: the picture must follow
        if (self.yaw_to - self.yaw).abs() > 0.05 || (self.pitch_to - self.pitch).abs() > 0.05 || (self.zoom_to - self.zoom).abs() > 0.001 {
            self.dirty = true;
        }
        self.yaw += (self.yaw_to - self.yaw) * k;
        self.pitch += (self.pitch_to - self.pitch) * k;
        self.zoom += (self.zoom_to - self.zoom) * k;
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
                Ok(Ready { look: l2, world, vt, vehicle, scheme })
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
            weather_to_scripts(&mut vehicle, &weather);
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
        Shown { look: r.look, scene, world: Some(world), vehicle: Some(vehicle), render: Some(render), trailers, centre, length, weather, lighting }
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
        let (yaw, pitch, zoom, focus) = (self.yaw, self.pitch, self.zoom, self.focus_now);
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
        let cam = Camera { position: pos, yaw: look_yaw, pitch: -pitch, roll: 0.0, fov_deg: fov, near: 0.2, far: 6000.0 };
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
        }
        renderer.render(&mut s.scene, target, w, h, &cam, &s.lighting);
        if standalone {
            renderer.instant_exposure = false;
        }
    }

    /// A bus is there to show.
    pub fn has_picture(&self) -> bool {
        self.shown.as_ref().map(|s| s.vehicle.is_some()).unwrap_or(false) && self.target.is_some()
    }

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

/// Tell the bus's scripts what the weather is doing, as the game tells them
/// (`weather_setup::apply_weather`): the precipitation type and rate, the state of the road
/// and the air. A few frames let them settle on it - the drop film on the windows builds up
/// over time rather than appearing at once.
fn weather_to_scripts(vehicle: &mut omsi_sim::VehicleInstance, weather: &omsi_content::weather::Weather) {
    crate::weather_setup::apply_weather(vehicle, weather, crate::weather_setup::initial_wetness(weather));
    for _ in 0..3 {
        vehicle.update(1.0 / 30.0);
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
