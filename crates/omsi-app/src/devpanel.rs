//! The development tools' buttons, over the top left of the picture while the vehicle
//! development mode is on (`crate::devmode`).
//!
//! A row of square icons and nothing else: what a developer reaches for again and again -
//! read the bus again, read it cold, watch its files, work on another one, copy it where it
//! may be edited - belongs under the cursor, not three pages deep in the game menu. The name
//! of a button appears beside the row while the cursor rests on it, so the row stays a row of
//! icons. A bus whose scripts did not compile gets a sixth button that says how many errors
//! there are and what the first of them is.
//!
//! It is drawn the way the touch controls are (`crate::touch`): `omsi-ui` into a painter of
//! its own, laid over the window after the scene is drawn. It is not part of the game's
//! interface (`crate::ui`), which draws its texts into textures and has no icons.

use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Weight};

use glam::Vec2;

/// What a button does. The actions themselves are the game menu's
/// (`App::page_action`), so a button and a menu line can never drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Act {
    Reload,
    Cold,
    Watch,
    Swap,
    Copy,
    Errors,
}

impl Act {
    /// The id `App::page_action` knows it by; `Errors` does nothing but say what it says.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Act::Reload => "reload",
            Act::Cold => "reloadcold",
            Act::Watch => "devwatch",
            Act::Swap => "swap",
            Act::Copy => "devcopy",
            Act::Errors => "noop",
        }
    }

    /// Material Symbols, as the launcher names them.
    fn icon(self) -> &'static str {
        match self {
            Act::Reload => "refresh",
            Act::Cold => "restart_alt",
            Act::Watch => "bolt",
            Act::Swap => "directions_bus",
            Act::Copy => "content_copy",
            Act::Errors => "error",
        }
    }
}

/// Panel colours: the launcher's dark grey, so the two interfaces of this program look like
/// one (`launcher::theme`).
const PANEL: Color = Color::rgba(22, 22, 22, 0.88);
const EDGE: Color = Color::rgba(255, 255, 255, 0.10);
const HOVER: Color = Color::rgba(52, 52, 52, 0.95);
const ACCENT: Color = Color::rgba(232, 160, 48, 1.0);
const DANGER: Color = Color::rgba(222, 78, 68, 1.0);
const TEXT: Color = Color::rgba(236, 236, 236, 1.0);
const TEXT_DIM: Color = Color::rgba(150, 150, 150, 1.0);

#[derive(Default)]
pub(crate) struct DevPanel {
    atlas: Option<Atlas>,
    painter: Painter,
    fonts: Option<Fonts>,
    gpu: Option<(Gpu, wgpu::TextureFormat)>,
    shot_gpu: Option<Gpu>,
    /// Where each button stands this frame (physical pixels) and what it does.
    buttons: Vec<(Rect, Act)>,
    /// The button the cursor rests on, whose name is shown beside the row.
    hover: Option<usize>,
}

impl DevPanel {
    /// The button under `(x, y)` (physical pixels), for a click.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<Act> {
        self.buttons.iter().find(|(r, _)| r.contains(Vec2::new(x, y))).map(|(_, a)| *a)
    }

    /// Whether the panel wants the cursor at all (so a click on it never reaches the cockpit
    /// or the camera behind it).
    pub(crate) fn over(&self, x: f32, y: f32) -> bool {
        self.hit(x, y).is_some()
    }

    pub(crate) fn drop_gpu(&mut self) {
        self.gpu = None;
        if let Some(a) = self.atlas.as_mut() {
            a.mark_all_dirty();
        }
    }

    /// Lay the row out and paint it. `u` is the interface's scale (`ui::size_factor` times
    /// the window's own), `cursor` where the mouse is in physical pixels.
    fn paint(&mut self, what: &[(Act, String, bool)], u: f32, cursor: (f32, f32)) {
        let atlas = self.atlas.get_or_insert_with(|| Atlas::new(1024));
        let fonts = self.fonts.get_or_insert_with(Fonts::new);
        atlas.begin_frame();
        self.painter = Painter::new();
        self.buttons.clear();
        self.hover = None;
        let pad = 8.0 * u;
        let size = 34.0 * u;
        let gap = 4.0 * u;
        let (x0, y0) = (14.0 * u, 14.0 * u);
        let w = pad * 2.0 + what.len() as f32 * size + (what.len().saturating_sub(1)) as f32 * gap;
        let panel = Rect::new(x0, y0, w, pad * 2.0 + size);
        self.painter.rounded(panel, 9.0 * u, PANEL);
        self.painter.rounded_border(panel, 9.0 * u, 1.0 * u, EDGE);
        let at = Vec2::new(cursor.0, cursor.1);
        for (i, (act, _, on)) in what.iter().enumerate() {
            let r = Rect::new(panel.x + pad + i as f32 * (size + gap), panel.y + pad, size, size);
            let hot = r.contains(at);
            if hot {
                self.hover = Some(i);
                self.painter.rounded(r, 6.0 * u, HOVER);
            } else if *on {
                self.painter.rounded(r, 6.0 * u, Color::rgba(44, 44, 44, 0.95));
            }
            let fg = match (act, on, hot) {
                (Act::Errors, _, _) => DANGER,
                (_, true, _) => ACCENT,
                (_, _, true) => TEXT,
                _ => TEXT_DIM,
            };
            self.painter.icon(atlas, act.icon(), r.center(), 19.0 * u, fg);
            self.buttons.push((r, *act));
        }
        // the name of the button under the cursor, beside the row - the row itself stays
        // icons, which is what it is for
        let Some(i) = self.hover else { return };
        let name = omsi_ui::tr(&what[i].1).into_owned();
        let tw = fonts.width(&name, 12.5 * u, Weight::Medium);
        let tip = Rect::new(panel.x, panel.bottom() + 6.0 * u, tw + 16.0 * u, 24.0 * u);
        self.painter.rounded(tip, 5.0 * u, PANEL);
        self.painter.rounded_border(tip, 5.0 * u, 1.0 * u, EDGE);
        self.painter.text_in(atlas, fonts, &name, 12.5 * u, Weight::Medium, tip, Align::Center, TEXT);
    }

    /// The row as a premultiplied RGBA picture, for the `shot` of an input script: that
    /// picture is the scene rendered again, without anything laid over the window
    /// (`crate::touch::picture` does the same for the on-screen controls).
    pub(crate) fn picture(&mut self, r: &omsi_render::Renderer, w: u32, h: u32) -> Option<Vec<u8>> {
        if self.painter.is_empty() {
            return None;
        }
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let atlas = self.atlas.as_mut()?;
        let gpu = self.shot_gpu.get_or_insert_with(|| Gpu::new(&r.device, format, 1, atlas.size));
        gpu.upload(&r.device, &r.queue, 0, &self.painter.verts);
        atlas.mark_all_dirty();
        gpu.upload_atlas(&r.queue, atlas);
        // (the window's pipeline has the atlas sent again next frame)
        atlas.mark_all_dirty();
        let tex = r.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dev tools shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let layers = [Layer::flat([0.0, 0.0, w as f32, h as f32], 0.0, 1.0)];
        let draws = [Draw { buffer: 0, range: 0..self.painter.len(), layer: 0, texture: 0 }];
        let mut enc = r.device.create_command_encoder(&Default::default());
        gpu.render(&r.device, &r.queue, &mut enc, &view, (w, h), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        let stride = (w * 4).div_ceil(256) * 256;
        let buf = r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (stride * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        r.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        omsi_render::wait_gpu(&r.device, None).ok();
        let data = buf.slice(..).get_mapped_range();
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            let row = &data[y * stride as usize..y * stride as usize + w as usize * 4];
            out[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(row);
        }
        Some(out)
    }

    /// The row painted over the window, after the scene (see `crate::touch::render`).
    pub(crate) fn render(&mut self, r: &omsi_render::Renderer, view: &wgpu::TextureView, w: u32, h: u32) {
        if self.painter.is_empty() {
            return;
        }
        let format = r.format();
        if self.gpu.as_ref().map(|g| g.1 != format).unwrap_or(true) {
            let size = self.atlas.as_ref().map(|a| a.size).unwrap_or(1024);
            self.gpu = Some((Gpu::new(&r.device, format, 1, size), format));
            if let Some(a) = self.atlas.as_mut() {
                a.mark_all_dirty();
            }
        }
        let (Some((gpu, _)), Some(atlas)) = (self.gpu.as_mut(), self.atlas.as_mut()) else { return };
        gpu.upload(&r.device, &r.queue, 0, &self.painter.verts);
        gpu.upload_atlas(&r.queue, atlas);
        let layers = [Layer::flat([0.0, 0.0, w as f32, h as f32], 0.0, 1.0)];
        let draws = [Draw { buffer: 0, range: 0..self.painter.len(), layer: 0, texture: 0 }];
        let mut enc = r.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dev tools") });
        gpu.render(&r.device, &r.queue, &mut enc, view, (w, h), None, &layers, &draws);
        r.queue.submit([enc.finish()]);
    }
}

impl crate::App {
    /// The buttons the panel shows for the bus loaded: what it does, its name for the
    /// hovering cursor, and whether it is "on" (the watch).
    fn dev_buttons(&self) -> Vec<(Act, String, bool)> {
        let tr = |t: &str| omsi_ui::tr(t).into_owned();
        let mut out = vec![
            (Act::Reload, tr("Reload this vehicle"), false),
            (Act::Cold, tr("Reload it cold"), false),
            (Act::Watch, tr("Reload when a file is saved"), self.dev.as_ref().is_some_and(|d| d.watch)),
            (Act::Swap, tr("Work on another vehicle"), false),
        ];
        // only where there is something to copy: a bus already outside the installation is
        // the one being edited
        if let Some(dir) = crate::devmode::vehicle_folder(self) {
            if crate::devmode::in_installation(&dir, &self.args.root) {
                out.push((Act::Copy, tr("Copy this vehicle to the content folder"), false));
            }
        }
        // the scripts that did not compile: the count and the first of them, where it can be
        // read without leaving the game (`Program::errors`)
        if let Some(p) = self.player.as_ref().filter(|p| !p.vehicle.ty.program.errors.is_empty()) {
            let e = &p.vehicle.ty.program.errors;
            let first = e.first().map(|e| format!("{e}")).unwrap_or_default();
            out.push((Act::Errors, format!("{} {}  ·  {first}", e.len(), tr("script errors")), false));
        }
        out
    }

    /// Per frame: the row laid out and painted while the mode is on, nothing otherwise.
    pub(crate) fn dev_panel_prepare(&mut self, w: u32, h: u32, dpi: f32) {
        // (nothing over a menu or a list of its own: they are modal, and the row would be
        // drawn over the card and take clicks meant for it)
        if !self.dev.as_ref().is_some_and(|d| d.on) || self.game_menu.is_some() || self.chooser.is_some() {
            self.dev_panel.painter = Painter::new();
            self.dev_panel.buttons.clear();
            return;
        }
        let u = dpi * crate::ui::size_factor(h as f32, dpi, self.settings.ui_scale, self.settings.ui_scale_window);
        let _ = w;
        let what = self.dev_buttons();
        let cursor = self.cursor;
        self.dev_panel.paint(&what, u, cursor);
    }

    /// A click on the row; true when it was the row's.
    pub(crate) fn dev_panel_click(&mut self, pressed: bool) -> bool {
        if !self.dev.as_ref().is_some_and(|d| d.on) {
            return false;
        }
        let (x, y) = self.cursor;
        let Some(act) = self.dev_panel.hit(x, y) else { return false };
        if !pressed {
            return true;
        }
        match act {
            // (it says what it is for in its name; the whole list goes to the log)
            Act::Errors => {
                if let Some(p) = self.player.as_ref() {
                    for e in &p.vehicle.ty.program.errors {
                        log::warn!("script: {e}");
                    }
                    let first = p.vehicle.ty.program.errors.first().map(|e| format!("{e}")).unwrap_or_default();
                    self.service_msg = Some((first, 8.0));
                }
            }
            other => {
                self.page_action(other.id());
            }
        }
        true
    }
}
