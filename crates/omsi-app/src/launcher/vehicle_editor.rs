//! The Vehicle Editor page: the chosen bus by itself, in the picture the game's own renderer
//! draws of it (see `showroom`), over the whole page. The mouse turns it and the wheel zooms
//! it, as on the Drive page - the picture is the one the showroom has already made for that
//! page, so opening this one costs nothing.
//!
//! Down the right stands a panel of buttons, each opening its choices beside it towards the
//! left, so the list never covers the very thing it changes. They set the same
//! `state.choice` fields the Drive page's day and weather step sets, which is what the
//! showroom watches: the bus is not read again, only its light (see `Showroom::update`).
//!
//! The picture is `Launcher::preview_full`, the stage the Drive page stands its bus on, and
//! `showroom_pointer` gives the wheel and the cursor to the showroom once the panel has had
//! the mouse. A drag on the panel is kept off the bus the same way, by `Ui::over_ui`.

use super::state::hhmm;
use super::theme::*;
use super::Launcher;
use omsi_launcher_lib as core;
use omsi_ui::Rect;
use super::showroom::EditorCam;
use winit::keyboard::KeyCode;

/// The panel of buttons over the picture, down its right.
const PANEL_W: f32 = 248.0;
const PANEL_PAD: f32 = 14.0;
/// A button of the panel: taller than a plain row, it carries its name and what it stands at.
const BUTTON_H: f32 = 46.0;

/// The light a time stands for. The hours themselves are the sun's, not fixed numbers: see
/// [`light_times`].
const LIGHTS: [&str; 3] = ["Day", "Dusk", "Night"];

/// The four views, in the order of the keys that take them: name, icon, and the key said in
/// the tooltip. `Driver` and `Pax` stand for whichever of the bus's eyes is in use.
const VIEWS: [(EditorCam, &str, &str, &str); 4] = [
    (EditorCam::Driver(0), "Driver's seat", "airline_seat_recline_normal", "F1"),
    (EditorCam::Pax(0), "Saloon", "groups", "F2"),
    (EditorCam::Outside, "Around the bus", "360", "F3"),
    (EditorCam::Free, "Free camera", "open_with", "F4"),
];

/// The view buttons: square, icon only, in a row at the top left of the picture. They are
/// the keys in another form, so they are kept as small as a control of this interface goes
/// - the page is there to look at the bus, not at its buttons.
const VIEW_BUTTON: f32 = 34.0;

/// The renderers to choose between, as the Settings page names them and in its order.
const MODES: [(&str, &str); 3] = [("vanilla", "Vanilla (as OMSI 2)"), ("vanilla_plus", "Vanilla+"), ("enhanced", "Enhanced")];

/// What the page keeps between frames: only the renderer it draws in, which starts as the
/// one the settings ask for and is changed here for looking, not saved - the Settings page
/// owns that choice, and a developer flicking between the three to compare a bus should not
/// find their game changed afterwards.
pub struct EditorView {
    pub graphics: String,
    /// How far the wipers have cleared their part of the panes (0..1, 1 = swept clean).
    pub wiped: f32,
}

impl Default for EditorView {
    fn default() -> Self {
        EditorView { graphics: core::graphics_mode(&crate::settings::Settings::load().graphics).to_string(), wiped: 1.0 }
    }
}

/// How fast a held arrow key turns the view (degrees a second), as the game turns its free
/// camera: 60 in yaw, 40 in pitch (`app_events`).
const TURN_YAW: f32 = 60.0;
const TURN_PITCH: f32 = 40.0;
/// How fast W/A/S/D/Q/E fly it (m/s), and what Shift multiplies that by. The game flies at
/// 30 m/s across a city; this stage is one bus long, so it goes at a walk and the boost is
/// the game's own factor of 5.
const FLY: f32 = 5.0;
const FLY_BOOST: f32 = 5.0;

/// A key pressed while this page has the keyboard. Returns whether it was one of ours.
///
/// The bindings are the game's own, as `Inputs/keyboard.cfg` has them: F1, F2 and F3 are
/// `view_set_driver`, `view_set_passenger` and `view_set_outside` (scan codes 59, 60, 61),
/// Numpad 4 and 6 are `view_interiorcam_minus` and `view_interiorcam_plus` - they step
/// through the eyes a bus gives - and Numpad 8 is `view_reset_all_directions`. The number
/// row is left alone: there it is doors, IBIS and the cash desk, nothing to do with a view.
pub fn view_key(l: &mut super::Launcher, code: KeyCode) -> bool {
    let view = match code {
        KeyCode::F1 => EditorCam::Driver(0),
        KeyCode::F2 => EditorCam::Pax(0),
        KeyCode::F3 => EditorCam::Outside,
        KeyCode::F4 => EditorCam::Free,
        KeyCode::Numpad4 | KeyCode::Numpad6 => {
            let by = if code == KeyCode::Numpad6 { 1 } else { -1 };
            let now = l.showroom.view();
            l.showroom.step_view(stepped(now, by, l.showroom.interior_count(now)));
            return true;
        }
        KeyCode::Numpad8 => {
            l.showroom.reset_turn();
            return true;
        }
        _ => return false,
    };
    l.showroom.set_view(view);
    true
}

/// The next of `n` eyes, `by` along and wrapping, as `view_interiorcam_plus/minus` step.
/// Outside, or where the bus gives no eye of that kind, nothing moves.
fn stepped(view: EditorCam, by: i32, n: usize) -> EditorCam {
    if n == 0 {
        return view;
    }
    let next = |i: usize| ((i as i32 + by).rem_euclid(n as i32)) as usize;
    match view {
        EditorCam::Driver(i) => EditorCam::Driver(next(i)),
        EditorCam::Pax(i) => EditorCam::Pax(next(i)),
        v => v,
    }
}

/// The keys held this frame fly and turn the camera, exactly the set the game flies its own
/// free camera with (`input_script::flies_free_camera`): W and S forward and back, A and D
/// to the sides, Q down and E (or Space) up, Shift to hurry, and the arrow keys to turn.
///
/// Only F4 flies, as only the game's F4 does. F1 and F2 sit at an eye the bus gives and F3
/// turns about it: a camera that could be flown out of those would not be the view any
/// more, and flying it under the floor is the first thing that happens.
fn fly_keys(l: &mut super::Launcher, dt: f32) {
    let down = |c: KeyCode| l.held.contains(&c);
    let axis = |a: KeyCode, b: KeyCode| down(a) as i32 as f32 - down(b) as i32 as f32;
    let (fwd, right) = (axis(KeyCode::KeyW, KeyCode::KeyS), axis(KeyCode::KeyD, KeyCode::KeyA));
    let up = (down(KeyCode::KeyE) || down(KeyCode::Space)) as i32 as f32 - down(KeyCode::KeyQ) as i32 as f32;
    if (fwd, right, up) != (0.0, 0.0, 0.0) {
        let boost = if down(KeyCode::ShiftLeft) || down(KeyCode::ShiftRight) { FLY_BOOST } else { 1.0 };
        l.showroom.fly(fwd, right, up, FLY * boost * dt);
    }
    let (dx, dy) = (axis(KeyCode::ArrowRight, KeyCode::ArrowLeft), axis(KeyCode::ArrowDown, KeyCode::ArrowUp));
    if (dx, dy) != (0.0, 0.0) {
        l.showroom.drag_by_degrees(dx * TURN_YAW * dt, dy * TURN_PITCH * dt);
    }
}

/// The page: the showroom over the whole of `r`, the buttons over its right.
///
/// A phone lays its pages out taller than its screen and scrolls them (`mobile::PAGE_H`), so
/// the picture is held to what the screen shows instead of growing with the page - a preview
/// taller than the window would be rendered at that size and mostly never seen.
pub fn draw(l: &mut Launcher, r: Rect) {
    let h = r.h.min(l.ui.size.y - r.y - 24.0).max(200.0);
    let page = Rect::new(r.x, r.y, r.w, h);
    // the panel is told before the picture is drawn, so a drag on it never turns the bus
    let rows = 4.0;
    let panel = Rect::new(
        page.right() - PANEL_PAD - PANEL_W,
        page.y + PANEL_PAD,
        PANEL_W,
        PANEL_PAD * 2.0 + rows * BUTTON_H + (rows - 1.0) * 8.0,
    );
    // the bus has the page to itself: the panel is a card in a corner, not a column beside
    // it, so the showroom frames it in the middle (`focus` 0.5) rather than to one side
    fly_keys(l, l.ui.dt);
    l.preview_full(page, 0.5);
    views(l, Rect::new(page.x + PANEL_PAD, page.y + PANEL_PAD, VIEW_BUTTON * 4.0 + 12.0 + 16.0, VIEW_BUTTON + 16.0));
    buttons(l, panel);
    l.showroom_pointer(page);
}

/// The row of view buttons over the top left of the picture. Which one is on follows the
/// camera, so the keys and the buttons never disagree.
fn views(l: &mut Launcher, panel: Rect) {
    l.ui.panel(panel);
    let now = l.showroom.view();
    for (i, (view, name, icon, key)) in VIEWS.iter().enumerate() {
        let r = Rect::new(panel.x + 8.0 + i as f32 * (VIEW_BUTTON + 4.0), panel.y + 8.0, VIEW_BUTTON, VIEW_BUTTON);
        let id = super::ui::id_of(&format!("editor-view-{i}"));
        let (hover, _, clicked) = l.ui.interact(id, r);
        // (an eye of the bus counts whichever of its eyes is in use)
        let on = std::mem::discriminant(&now) == std::mem::discriminant(view);
        if on {
            l.ui.p().rounded(r, 6.0, SELECTED);
        } else if hover {
            l.ui.p().rounded(r, 6.0, HOVER);
        }
        let c = if on { ACCENT } else if hover { TEXT } else { TEXT_DIM };
        l.ui.icon(icon, r.center(), 19.0, c);
        l.ui.tooltip(r, &format!("{} ({key})", omsi_ui::tr(name)));
        if clicked {
            l.showroom.set_view(*view);
        }
    }
}

fn buttons(l: &mut Launcher, panel: Rect) {
    l.ui.panel(panel);
    let w = panel.w - PANEL_PAD * 2.0;
    let x = panel.x + PANEL_PAD;
    let mut y = panel.y + PANEL_PAD;
    weather(l, Rect::new(x, y, w, BUTTON_H));
    y += BUTTON_H + 8.0;
    time(l, Rect::new(x, y, w, BUTTON_H));
    y += BUTTON_H + 8.0;
    graphics(l, Rect::new(x, y, w, BUTTON_H));
    y += BUTTON_H + 8.0;
    wipers(l, Rect::new(x, y, w, BUTTON_H));
}

/// The wiped area: how much of the water the wipers have taken off their part of the panes.
///
/// `Rain_Window_Wiped_Wetness` is what the stock `wiper.osc` works on - each sweep takes
/// `|Rain_Wiper_Pos - Rain_Wiper_Pos_Prev| * rain_wiper_eff` off it, down to 0 - and the
/// slider stands it anywhere between swept clean and as wet as the rest of the glass. The
/// note under it is the curve that makes it visible: the layer is drawn at
/// `min(1, wetness * 1.8)` (`omsi_sim::vehicle`), so it is already fully opaque at a wetness
/// of 0.56 and the top of the slider's travel shows no further change.
fn wipers(l: &mut Launcher, r: Rect) {
    let mut percent = l.editor.wiped * 100.0;
    let wet = (1.0 - l.editor.wiped) * l.showroom.wetness();
    let note = format!("{} {:.0} %", omsi_ui::tr("Layer opacity"), (wet * 1.8).min(1.0) * 100.0);
    if l.ui.menu_slider("editor-wipers", r, "Wiped area", &mut percent, 0.0, 100.0, 1.0, " %", &note, "water_drop") {
        l.editor.wiped = (percent / 100.0).clamp(0.0, 1.0);
    }
}

/// The renderer button: the three the Settings page offers, switched here for this page
/// alone (see [`EditorView`]). Switching only changes `Look`, so the bus is not read again -
/// the picture is drawn once more in the other renderer.
fn graphics(l: &mut Launcher, r: Rect) {
    let labels: Vec<String> = MODES.iter().map(|(_, n)| (*n).to_string()).collect();
    let mut sel = MODES.iter().position(|(v, _)| *v == l.editor.graphics).unwrap_or(1);
    let value = omsi_ui::tr(MODES[sel.min(MODES.len() - 1)].1).to_string();
    if l.ui.menu_button("editor-graphics", r, "Graphics", &value, "palette", &mut sel, &labels) {
        l.editor.graphics = MODES[sel].0.to_string();
    }
}

/// The weather button: the installed weathers, without the ones that are not a scene a
/// developer can look at - the custom editor, the airport's live report and the cycle that
/// changes while the game runs. The season is not asked either, as the Drive page asks it
/// (`State::weather_fits`): snow in May is exactly the thing one opens this page for.
fn weather(l: &mut Launcher, r: Rect) {
    let list: Vec<(String, String)> = l.state.weathers.iter().map(|w| (w.file.clone(), w.name.clone())).collect();
    if list.is_empty() {
        l.ui.label(Rect::new(r.x + 4.0, r.y, r.w - 8.0, r.h), "No weathers installed");
        return;
    }
    let names: Vec<String> = list.iter().map(|(_, n)| n.clone()).collect();
    let chosen = l.state.choice.weather.clone();
    // (none of them when the map's own weather is left, or a cycle or a report is chosen on
    // the Drive page: the button then says so and the list ticks nothing)
    let mut sel = list.iter().position(|(f, _)| *f == chosen).unwrap_or(usize::MAX);
    let value = match names.get(sel) {
        Some(n) => n.clone(),
        None if chosen.is_empty() => omsi_ui::tr("The map's own").to_string(),
        None => omsi_ui::tr("Not one of these").to_string(),
    };
    if l.ui.menu_button("editor-weather", r, "Weather", &value, "partly_cloudy_day", &mut sel, &names) {
        l.state.choice.weather = list[sel].0.clone();
        l.state.touched();
    }
}

/// The time button: day, dusk and night, as hours of the map and the date rather than as
/// three fixed numbers (see [`light_times`]). The value underneath is the clock time they
/// come to, so it is plain that a northern map's dusk in December is not its dusk in June.
fn time(l: &mut Launcher, r: Rect) {
    let times = light_times(&l.state.choice.date, &omsi_sim::daylight::place());
    let now = l.state.choice.time;
    let labels: Vec<String> = LIGHTS.iter().zip(times).map(|(n, t)| format!("{} · {}", omsi_ui::tr(n), hhmm(t as f64 * 60.0))).collect();
    // the one it stands at: within a quarter of an hour of it, so that turning the hour a
    // little on the Drive page does not make the button lie
    let mut sel = times.iter().position(|t| (t - now).abs() <= 15).unwrap_or(usize::MAX);
    let value = match sel {
        k if k < LIGHTS.len() => omsi_ui::tr(LIGHTS[k]).to_string(),
        _ => hhmm(now as f64 * 60.0),
    };
    if l.ui.menu_button("editor-time", r, "Time", &value, "wb_twilight", &mut sel, &labels) {
        l.state.choice.time = times[sel];
        l.state.touched();
    }
}

/// Day, dusk and night as minutes of the day, from where the sun itself stands: the highest
/// it gets, four degrees under the horizon on its way down (civil twilight, the light OMSI's
/// own night textures come on in), and the lowest it gets. The place is the one the
/// showroom's scene set (`omsi_sim::daylight::place`), so each map and each date gets its
/// own hours instead of three numbers that are dusk in Berlin in March and nowhere else.
/// (`place` is handed over rather than read here, so a test can ask for another sky.)
fn light_times(date: &str, place: &omsi_sim::daylight::SunPlace) -> [i32; 3] {
    let (y, m, d) = super::ui::parse_date(date);
    let mut clock = omsi_sim::SimClock::default();
    clock.set_date(y, m as i32, d as i32);
    let altitude = |minute: i32| {
        let mut c = clock.clone();
        c.time = minute as f64 * 60.0;
        omsi_sim::daylight::sun_position(&c, place).0
    };
    let minutes: Vec<i32> = (0..1440).step_by(2).collect();
    let by = |a: &&i32, b: &&i32| altitude(**a).total_cmp(&altitude(**b));
    let day = *minutes.iter().max_by(by).unwrap_or(&720);
    let night = *minutes.iter().min_by(by).unwrap_or(&0);
    // after the sun is highest, so it is the evening twilight and not the morning one
    let dusk = minutes
        .iter()
        .filter(|&&t| t > day)
        .min_by(|a, b| (altitude(**a) + 4.0).abs().total_cmp(&(altitude(**b) + 4.0).abs()))
        .copied()
        .unwrap_or(day);
    [day, dusk, night]
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_sim::daylight::SunPlace;

    /// The three times follow the sun of the place and the date, not a fixed clock: Berlin's
    /// midsummer dusk is late in the evening, its midwinter dusk hours earlier, and noon
    /// stands where the sun is highest. A map far to the south has its own, closer together.
    #[test]
    fn day_dusk_and_night_follow_the_sun_of_the_map_and_the_date() {
        let berlin = SunPlace::default();
        let [day, dusk, night] = light_times("1989-06-21", &berlin);
        assert!((660..=780).contains(&day), "midday at {day} minutes");
        assert!(dusk > day, "dusk comes after the sun is highest");
        assert!((1200..=1380).contains(&dusk), "midsummer dusk at {dusk} minutes");
        assert!(night < 360 || night > 1380, "the darkest hour is around midnight, not {night}");

        let [_, winter_dusk, _] = light_times("1989-12-21", &berlin);
        assert!(winter_dusk + 180 < dusk, "midwinter dusk ({winter_dusk}) must be hours before midsummer's ({dusk})");

        // the equator: the sun drops fast, so dusk follows sunset closely all year
        let quito = SunPlace { latitude: -0.2, longitude: -78.5, timezone: -5.0, dst: Vec::new() };
        let [_, a, _] = light_times("1989-06-21", &quito);
        let [_, b, _] = light_times("1989-12-21", &quito);
        assert!((a - b).abs() < 60, "dusk hardly moves over the year there: {a} and {b}");
    }

    /// Stepping through a bus's eyes wraps both ways and stands still where there is none.
    #[test]
    fn the_interior_cameras_step_round() {
        assert_eq!(stepped(EditorCam::Driver(0), 1, 3), EditorCam::Driver(1));
        assert_eq!(stepped(EditorCam::Driver(2), 1, 3), EditorCam::Driver(0), "forwards past the last");
        assert_eq!(stepped(EditorCam::Driver(0), -1, 3), EditorCam::Driver(2), "back past the first");
        assert_eq!(stepped(EditorCam::Pax(1), -1, 2), EditorCam::Pax(0));
        assert_eq!(stepped(EditorCam::Pax(0), 1, 0), EditorCam::Pax(0), "a bus with no such eye");
        assert_eq!(stepped(EditorCam::Outside, 1, 3), EditorCam::Outside, "outside has none to step");
        assert_eq!(stepped(EditorCam::Free, 1, 3), EditorCam::Free, "nor the free camera");
    }

    /// The names the time button offers are keys of the translation tables, so the page is
    /// not English where the rest of the launcher is not.
    #[test]
    fn the_light_names_are_the_ones_the_translations_hold() {
        let yml = include_str!("../../locales/app.yml");
        for name in LIGHTS {
            assert!(yml.contains(&format!("\"{name}\":")), "{name} has no translations");
        }
    }
}
