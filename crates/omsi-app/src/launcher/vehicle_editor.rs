//! The Vehicle Editor page: the chosen bus by itself, in the picture the game's own renderer
//! draws of it (see `showroom`), over the whole page. The mouse turns it and the wheel zooms
//! it, as on the Drive page's card - `Launcher::preview` does both, and the picture is the
//! one the showroom has already made for that page, so opening this one costs nothing.
//!
//! Down the right stands a panel of buttons, each opening its choices beside it towards the
//! left, so the list never covers the very thing it changes. They set the same
//! `state.choice` fields the Drive page's Time & weather step sets, which is what the
//! showroom watches: the bus is not read again, only its light (see `Showroom::update`).

use super::state::hhmm;
use super::Launcher;
use omsi_ui::Rect;

/// The panel of buttons over the picture, down its right.
const PANEL_W: f32 = 248.0;
const PANEL_PAD: f32 = 14.0;
/// A button of the panel: taller than a plain row, it carries its name and what it stands at.
const BUTTON_H: f32 = 46.0;

/// The light a time stands for. The hours themselves are the sun's, not fixed numbers: see
/// [`light_times`].
const LIGHTS: [&str; 3] = ["Day", "Dusk", "Night"];

/// The page: the showroom over the whole of `r`, the buttons over its right.
///
/// A phone lays its pages out taller than its screen and scrolls them (`mobile::PAGE_H`), so
/// the picture is held to what the screen shows instead of growing with the page - a preview
/// taller than the window would be rendered at that size and mostly never seen.
pub fn draw(l: &mut Launcher, r: Rect) {
    let h = r.h.min(l.ui.size.y - r.y - 24.0).max(200.0);
    let page = Rect::new(r.x, r.y, r.w, h);
    // the panel is told before the picture is drawn, so a drag on it never turns the bus
    let rows = 2.0;
    let panel = Rect::new(
        page.right() - PANEL_PAD - PANEL_W,
        page.y + PANEL_PAD,
        PANEL_W,
        PANEL_PAD * 2.0 + rows * BUTTON_H + (rows - 1.0) * 8.0,
    );
    l.preview_block = Some(panel);
    l.preview(page);
    buttons(l, panel);
}

fn buttons(l: &mut Launcher, panel: Rect) {
    l.ui.panel(panel);
    let w = panel.w - PANEL_PAD * 2.0;
    let x = panel.x + PANEL_PAD;
    let mut y = panel.y + PANEL_PAD;
    weather(l, Rect::new(x, y, w, BUTTON_H));
    y += BUTTON_H + 8.0;
    time(l, Rect::new(x, y, w, BUTTON_H));
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
