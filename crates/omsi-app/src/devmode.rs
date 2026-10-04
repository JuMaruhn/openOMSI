//! The vehicle development mode: the game itself as the workshop for a bus.
//!
//! A bus is developed by driving it - its doors, switches, IBIS, sound and physics only
//! answer in the game - so the place to work on one is the game, not a picture of it in the
//! launcher. The mode gathers what that work needs on one page of the game menu: the bus to
//! work on, the light to look at it in, a script variable written by hand, and the vehicle
//! read again from its files (`App::reload_driven_vehicle`, #728). The world is quietened
//! while it is on - no traffic, no passengers, a clock that stands still - and put back the
//! way it was when it is left, because none of that is what is being looked at.
//!
//! Ctrl+Shift+D, the game menu's *Vehicle development...*, or `--dev-vehicle` at the start.
//!
//! **The original installation is never written to.** A bus that lives there cannot be
//! edited, so the page offers to copy its folder into the content folder, which the game
//! reads first (`omsi_cfg` content roots); the copy is the one loaded from then on. Whole
//! folders are copied, never single files: a vehicle package is read from one root only
//! (`omsi_cfg::vehicle_package`, `docs/FORMATS.md`), so half a bus in the content folder
//! would hide the other half rather than complete it.

use crate::game_lists as gl;
use crate::App;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

/// What the mode quietened when it was entered, to put back when it is left.
struct Quieted {
    traffic: f32,
    pax: f32,
    speed: f32,
}

pub(crate) struct DevMode {
    quiet: Option<Quieted>,
    /// The script variable the page writes, and the value typed for it.
    pub(crate) var: String,
    pub(crate) value: String,
    /// Which of the two the typing goes into.
    pub(crate) typing_value: bool,
    /// A copy of the vehicle's folder into the content folder, under way on a worker.
    copying: Option<Receiver<Result<PathBuf, String>>>,
    /// What that copy is called while it runs, for the page to say.
    pub(crate) copying_name: String,
    /// Watching the vehicle's files: saving one reads the bus again (see [`watch`]).
    pub(crate) watch: bool,
    /// The newest change time seen under its folder, the look that is under way on a worker,
    /// and when the last one was asked for. None while nothing has been seen yet - the first
    /// answer only sets the mark, it never reloads.
    stamp: Option<SystemTime>,
    looking: Option<Receiver<Option<SystemTime>>>,
    looked: Option<Instant>,
}

/// How often the vehicle's folder is looked at. The Lua plugin host looks once a second and
/// that has proved to be soon enough for a save to feel immediate (`omsi-plugin::lua`); a
/// vehicle folder holds a few thousand files, so it is looked at no more often and on a
/// worker - 1600 files cost some milliseconds, which is a frame.
const WATCH_EVERY: Duration = Duration::from_secs(1);

impl Default for DevMode {
    fn default() -> Self {
        DevMode { quiet: None, var: String::new(), value: String::new(), typing_value: false, copying: None, copying_name: String::new(), watch: false, stamp: None, looking: None, looked: None }
    }
}

impl DevMode {
    pub(crate) fn copying(&self) -> bool {
        self.copying.is_some()
    }
}

/// Every variable and string variable of a vehicle, by name: what a reload carries over so
/// the bus comes back running (`VehicleInstance::restore_script_state` puts them back by
/// name, which is the only way that survives a varlist gaining or losing a line - the places
/// in the `Program` move with it). A situation is kept the same way (`situation`).
pub(crate) fn script_state_of(v: &omsi_sim::VehicleInstance) -> (Vec<(String, f32)>, Vec<(String, String)>) {
    let p = &v.ty.program;
    let vars = p.var_names.iter().enumerate().map(|(i, n)| (n.clone(), v.state.vars.get(i).copied().unwrap_or(0.0))).collect();
    let strs = p.str_var_names.iter().enumerate().map(|(i, n)| (n.clone(), v.state.str_vars.get(i).cloned().unwrap_or_default())).collect();
    (vars, strs)
}

/// The folder a bus's files live in, and whether it can be edited - that is, whether it
/// lies outside the original installation (the content folder, a mod, an archive).
pub(crate) fn vehicle_folder(app: &App) -> Option<PathBuf> {
    app.player.as_ref().map(|p| p.vehicle.ty.def.dir().to_path_buf())
}

/// Whether `dir` lies inside the original installation, which openOMSI never writes to.
pub(crate) fn in_installation(dir: &Path, root: &Path) -> bool {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    real(dir).starts_with(real(root))
}

/// Where a copy of `dir` would go: the same `Vehicles/<folder>` under the content folder.
pub(crate) fn copy_target(dir: &Path, content: &Path) -> PathBuf {
    let rel: PathBuf = dir
        .components()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    content.join(rel)
}

/// The rows of the development page.
pub(crate) fn pages(app: &App) -> Vec<(&'static str, Vec<(String, String)>)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let dev = app.dev.as_ref();
    let mut bus: Vec<(String, String)> = Vec::new();
    match app.player.as_ref() {
        Some(p) => {
            let name = format!("{} {}", p.vehicle.ty.def.manufacturer.trim(), p.vehicle.ty.def.type_name.trim());
            bus.push((gl::row("Vehicle", 'i', name.trim(), &p.vehicle.ty.def.path.to_string_lossy(), None), "noop".into()));
            bus.push(gl::button("Reload this vehicle", "Reload", "Read its files again (.bus, model, sound configuration, scripts, textures and meshes) and drive on with the state it has", "reload"));
            bus.push(gl::button("Reload it cold", "Cold", "The same, but the bus comes back as it is first put down - engine off, every variable at its start. What to try when a change does not seem to take", "reloadcold"));
            let on = dev.is_some_and(|d| d.watch);
            bus.push(gl::button(
                "Reload when a file is saved",
                if on { "On" } else { "Off" },
                "Look at the vehicle's folder every second and read the bus again as soon as anything in it has been saved - a script, a texture, a mesh",
                "devwatch",
            ));
            bus.push(gl::button("Work on another vehicle", "Swap", "Put another vehicle in this one's place and drive it", "swap"));
            // where its files are, and whether they may be edited at all
            let dir = p.vehicle.ty.def.dir().to_path_buf();
            let installed = in_installation(&dir, &app.args.root);
            if let Some(d) = dev.filter(|d| d.copying()) {
                bus.push((gl::row("Copy", 'i', &tr("Copying..."), &d.copying_name, None), "noop".into()));
            } else if installed {
                bus.push(gl::button(
                    "Copy this vehicle to the content folder",
                    "Copy",
                    "Its files are in the original installation, which is never written to. The copy is read before it, and is the one to edit",
                    "devcopy",
                ));
            } else {
                bus.push((gl::row("Files", 'i', &tr("Can be edited"), &dir.to_string_lossy(), None), "noop".into()));
            }
        }
        None => bus.push((gl::row("Vehicle", 'i', &tr("None - you are on foot"), "", None), "noop".into())),
    }

    // a script variable by hand: what the launcher's wiper slider did, for every variable
    let mut vars: Vec<(String, String)> = Vec::new();
    if app.player.is_some() {
        // the field being typed shows what is in the keyboard's hands, with a caret; the
        // other one what it stands at
        let editing = app.menu_edit.as_ref().and(dev.map(|d| d.typing_value));
        let typed = |s: String, mine: bool| match editing {
            Some(e) if e == mine => format!("{}_", app.menu_edit.clone().unwrap_or_default()),
            _ if s.is_empty() => tr("(none)"),
            _ => s,
        };
        let (name, value) = dev.map(|d| (d.var.clone(), d.value.clone())).unwrap_or_default();
        let kind = |mine: bool| if editing == Some(mine) { 'E' } else { 'e' };
        vars.push((gl::row("Variable", kind(false), &typed(name.clone(), false), "The name of a script variable of the vehicle (press Enter to type)", None), "devvar".into()));
        vars.push((gl::row("Value", kind(true), &typed(value, true), "What to write into it (press Enter to type)", None), "devvalue".into()));
        let now = app.player.as_ref().and_then(|p| p.vehicle.var(name.trim())).map(|v| format!("{v}")).unwrap_or_else(|| tr("not a variable of this bus"));
        vars.push((gl::row("It stands at", 'i', &now, "", None), "noop".into()));
        vars.push(gl::button("Write it", "Set", "Write the value into the variable now", "devset"));
    }

    // the world, quietened: the rows of the World window, so the one page is enough
    let mut world: Vec<(String, String)> = Vec::new();
    world.extend(gl::slider_row(app, "traffic", "Traffic", "How many vehicles drive around the map.", &|v| format!("{} vehicles", v as i64)));
    world.extend(gl::slider_row(app, "pax", "Passengers", "How many passengers wait at the stops and ride.", &|v| format!("{:.0} %", v * 100.0)));
    world.extend(gl::slider_row(app, "speed", "Time speed", "How fast the clock runs.", &|v| format!("{v} x")));
    world.push(gl::button("Weather", "Next", "The next installed weather", "weather"));
    world.push(gl::button("Weather and time in full", "Open", "The World options, where every weather value and the clock are set", "devworld"));

    vec![("Vehicle", bus), ("Variable", vars), ("Files", files(app)), ("Scripts", errors(app)), ("World", world)]
}

/// What the bus is made of: the files it was read from, and the meshes with the variable
/// that moves each. The list a reload goes over, and the one a watch would watch.
fn files(app: &App) -> Vec<(String, String)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(p) = app.player.as_ref() else { return out };
    let (def, model) = (&p.vehicle.ty.def, &p.vehicle.ty.model);
    let dir = def.dir().to_path_buf();
    // (under the vehicle's own folder, which is what the rows have room for)
    let short = |path: &Path| path.strip_prefix(&dir).unwrap_or(path).to_string_lossy().to_string();
    let named = |rel: &Option<String>| rel.clone().unwrap_or_default();
    out.push((gl::row(&tr("Vehicle file"), 'i', &short(&def.path), &dir.to_string_lossy(), None), "noop".into()));
    for (what, name) in [("Model", named(&def.model)), ("Sound", named(&def.sound)), ("Paths", named(&def.paths)), ("Passenger cabin", named(&def.passenger_cabin))] {
        if !name.trim().is_empty() {
            out.push((gl::row(what, 'i', name.trim(), "", None), "noop".into()));
        }
    }
    let list = |what: &str, files: &[std::path::PathBuf], out: &mut Vec<(String, String)>| {
        for (i, f) in files.iter().enumerate() {
            let label = if i == 0 { what.to_string() } else { String::new() };
            out.push((gl::row(&label, 'i', &short(f), "", None), "noop".into()));
        }
    };
    list(&tr("Scripts"), &def.scripts.scripts, &mut out);
    list(&tr("Constants"), &def.scripts.constfiles, &mut out);
    list(&tr("Variable lists"), &def.scripts.varlists, &mut out);
    list(&tr("String lists"), &def.scripts.stringvarlists, &mut out);
    // the meshes, each with the variable that moves it: the map between a model and a script
    out.push((gl::row(&tr("Meshes"), 'i', &format!("{}", model.meshes.len()), "", None), "noop".into()));
    for m in model.meshes.iter().filter(|m| !m.file.trim().is_empty()) {
        let moved: Vec<&str> = m.animations.iter().map(|a| a.variable.trim()).filter(|v| !v.is_empty()).collect();
        let value = if moved.is_empty() { tr("still") } else { moved.join(", ") };
        out.push((gl::row(m.file.trim(), 'i', &value, "", None), "noop".into()));
    }
    out
}

/// What the compiler made of the scripts: every error with its file and line, as
/// `Program::errors` collects them. They were only ever in the log before, where a developer
/// working on a script had to go and look for them.
fn errors(app: &App) -> Vec<(String, String)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(p) = app.player.as_ref() else { return out };
    let program = &p.vehicle.ty.program;
    if program.errors.is_empty() {
        out.push((gl::row(&tr("Scripts"), 'i', &tr("No errors"), &tr("Every script of this bus compiled"), None), "noop".into()));
        return out;
    }
    out.push((gl::row(&tr("Script errors"), 'i', &format!("{}", program.errors.len()), "", None), "noop".into()));
    // (a bus with a broken script can have hundreds: the first of them are the ones to read)
    for e in program.errors.iter().take(40) {
        let file = e.file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        out.push((gl::row(&format!("{file}:{}", e.line), 'i', &e.message, &e.file.to_string_lossy(), None), "noop".into()));
    }
    out
}

/// Enter or leave the mode. Entering quietens the world, leaving puts it back.
pub(crate) fn toggle(app: &mut App) {
    if let Some(mut d) = app.dev.take() {
        if let Some(q) = d.quiet.take() {
            gl::set_slider(app, "traffic", q.traffic);
            gl::set_slider(app, "pax", q.pax);
            gl::set_slider(app, "speed", q.speed);
        }
        app.service_msg = Some((omsi_ui::tr("Vehicle development off").into_owned(), 3.0));
        return;
    }
    app.dev = Some(DevMode::default());
    app.service_msg = Some((omsi_ui::tr("Vehicle development on - the game menu has its page, Ctrl+Shift+D leaves it").into_owned(), 8.0));
}

/// Write the typed value into the typed variable.
pub(crate) fn set_variable(app: &mut App) {
    let Some((name, value)) = app.dev.as_ref().map(|d| (d.var.trim().to_string(), d.value.trim().to_string())) else { return };
    if name.is_empty() {
        app.service_msg = Some((omsi_ui::tr("Name a variable first").into_owned(), 3.0));
        return;
    }
    let Ok(v) = value.parse::<f32>() else {
        app.service_msg = Some((format!("{}: {value}", omsi_ui::tr("That is not a number")), 3.0));
        return;
    };
    let Some(p) = app.player.as_mut() else { return };
    if p.vehicle.var(&name).is_none() {
        app.service_msg = Some((format!("{} {name}", omsi_ui::tr("This bus has no variable")), 3.0));
        return;
    }
    p.vehicle.set_var(&name, v);
    app.service_msg = Some((format!("{name} = {v}"), 3.0));
}

/// Copy the driven vehicle's folder into the content folder, on a worker: a pack is large
/// and the game must not stand still for it. The answer is picked up by [`tick`].
pub(crate) fn start_copy(app: &mut App) {
    let Some(dir) = vehicle_folder(app) else { return };
    let Some(content) = crate::startup::content_dir() else {
        app.service_msg = Some((omsi_ui::tr("There is no content folder to copy into").into_owned(), 4.0));
        return;
    };
    if app.dev.as_ref().is_some_and(|d| d.copying()) {
        return;
    }
    let out = copy_target(&dir, &content);
    if in_installation(&out, &app.args.root) {
        app.service_msg = Some((omsi_ui::tr("The copy would land in the original installation: not written").into_owned(), 5.0));
        return;
    }
    if out.exists() {
        app.service_msg = Some((format!("{} {}", omsi_ui::tr("It is already there:"), out.display()), 5.0));
        return;
    }
    let name = out.display().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = copy_folder(&dir, &out).map(|_| out.clone()).map_err(|e| format!("{e}"));
        let _ = tx.send(r);
    });
    if let Some(d) = app.dev.as_mut() {
        d.copying = Some(rx);
        d.copying_name = name.clone();
    }
    app.service_msg = Some((format!("{} {name}", omsi_ui::tr("Copying the vehicle to")), 6.0));
}

/// Nothing but the bus: the traffic and the people away and the clock still, the moment
/// there is a world to quieten (`--dev-vehicle` is on before one is loaded). What they
/// stood at is kept and put back when the mode is left - they are the player's settings,
/// not the mode's.
fn quieten(app: &mut App) {
    if app.dev.as_ref().is_none_or(|d| d.quiet.is_some()) {
        return;
    }
    let now = |id: &str| gl::slider_now(app, id);
    let (Some(traffic), Some(pax), Some(speed)) = (now("traffic"), now("pax"), now("speed")) else { return };
    if let Some(d) = app.dev.as_mut() {
        d.quiet = Some(Quieted { traffic, pax, speed });
    }
    gl::set_slider(app, "traffic", 0.0);
    gl::set_slider(app, "pax", 0.0);
    gl::set_slider(app, "speed", 0.0);
}

/// The newest change time of anything under `dir`. A vehicle is a folder of files that name
/// each other - the `.bus`, the model and its meshes, the scripts, the textures - and a save
/// may touch any of them, so the folder is taken as a whole rather than the files the bus
/// happens to name today (a texture added while working on it would be in neither list).
fn newest_change(dir: &Path) -> Option<SystemTime> {
    fn walk(dir: &Path, best: &mut Option<SystemTime>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, best);
            } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                *best = Some(best.map_or(t, |b: SystemTime| b.max(t)));
            }
        }
    }
    let mut best = None;
    walk(dir, &mut best);
    best
}

/// Saving a file of the vehicle reads the bus again, with the state it had - the loop this
/// mode is for: change a script in whatever editor, save, and the bus in front of you is the
/// changed one, still running. The folder is looked at on a worker so that the look costs the
/// picture nothing, and the first answer only sets the mark: nothing is read because the mode
/// was turned on.
fn watch(app: &mut App) {
    let Some(d) = app.dev.as_ref() else { return };
    if !d.watch || d.copying() || app.player.is_none() {
        return;
    }
    // what the worker found, if it has finished
    let found = d.looking.as_ref().and_then(|rx| rx.try_recv().ok());
    if let Some(now) = found {
        let Some(d) = app.dev.as_mut() else { return };
        d.looking = None;
        let before = d.stamp;
        d.stamp = now;
        // (the first look only marks where things stood)
        if before.is_some() && now.is_some() && before != now {
            // (the mark is set anew after the reload: the bus may come from another folder
            // now, and the files are not touched by the reading itself)
            d.stamp = None;
            app.service_msg = Some((omsi_ui::tr("A file changed: reading the vehicle again").into_owned(), 3.0));
            app.reload_driven_vehicle_keeping(true);
            // (a page of the mode that is open shows the bus as it is now - the Scripts page
            // above all, which is where a save that does not compile is read)
            if app.list_kind.is_some() {
                app.refresh_list();
            }
            return;
        }
    }
    let Some(d) = app.dev.as_ref() else { return };
    if d.looking.is_some() || d.looked.is_some_and(|t| t.elapsed() < WATCH_EVERY) {
        return;
    }
    let Some(dir) = vehicle_folder(app) else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(newest_change(&dir));
    });
    if let Some(d) = app.dev.as_mut() {
        d.looking = Some(rx);
        d.looked = Some(Instant::now());
    }
}

/// Per frame: quieten the world once there is one, pick the copy up when it is done, and
/// read the bus again when one of its files has been saved.
pub(crate) fn tick(app: &mut App) {
    quieten(app);
    watch(app);
    let done = match app.dev.as_mut().and_then(|d| d.copying.as_ref()) {
        Some(rx) => match rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("the copy did not finish".to_string())),
        },
        None => None,
    };
    let Some(r) = done else { return };
    if let Some(d) = app.dev.as_mut() {
        d.copying = None;
        d.copying_name.clear();
    }
    match r {
        Ok(out) => {
            // the content folder is read before the installation, so the copy is what the
            // next read of the bus finds: the listings must forget what they saw
            omsi_cfg::content_changed();
            log::info!("vehicle development: the vehicle was copied to {}", out.display());
            app.service_msg = Some((format!("{} {} - {}", omsi_ui::tr("Copied to"), out.display(), omsi_ui::tr("reload the vehicle to work on the copy")), 10.0));
        }
        Err(e) => {
            log::warn!("vehicle development: the copy failed: {e}");
            app.service_msg = Some((format!("{}: {e}", omsi_ui::tr("The copy failed")), 6.0));
        }
    }
}

/// Copy a folder with everything in it. (`std::fs` has none, and a vehicle folder is a few
/// hundred megabytes of textures and meshes.)
fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_folder(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy keeps the `Vehicles/<pack>` the game looks for it under, so the content
    /// folder's copy stands where the installation's folder stood.
    #[test]
    fn a_copy_keeps_the_two_folders_the_game_looks_it_up_by() {
        let dir = Path::new("/games/OMSI 2/Vehicles/MAN_SD200");
        let out = copy_target(dir, Path::new("/home/me/openOMSI"));
        assert_eq!(out, Path::new("/home/me/openOMSI/Vehicles/MAN_SD200"));
    }

    /// What a reload carries over is read and put back **by name**: a fresh instance of the
    /// same bus comes back with the value it had, which is what lets the engine go on running
    /// while a script is worked on. (Needs an installation; without one it says so and stops.)
    #[test]
    fn the_state_a_reload_carries_is_put_back_by_name() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_SD200/MAN_SD80.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = std::sync::Arc::new(omsi_sim::VehicleType::load(&root, &bus).expect("the bus loads"));
        let new = |ty: std::sync::Arc<omsi_sim::VehicleType>| omsi_sim::VehicleInstance::new(ty, omsi_sim::VehicleHost::new(omsi_sim::SimClock::default()));
        let mut v = new(ty.clone());
        v.set_var("Velocity", 12.5);
        let (vars, strs) = script_state_of(&v);
        assert!(vars.iter().any(|(n, x)| n.eq_ignore_ascii_case("Velocity") && *x == 12.5), "the value is in the snapshot");
        let mut again = new(ty);
        assert_ne!(again.var("Velocity"), Some(12.5), "a fresh bus starts cold");
        again.restore_script_state(&vars, &strs);
        assert_eq!(again.var("Velocity"), Some(12.5), "and comes back with what was kept");
    }

    /// A coupled part's own state is where its turning axle stands, and a reload carries it:
    /// an articulated bus read again in a bend comes back bent, not snapped straight. The
    /// bend is made here by moving the axle, which is what driving round a corner does to it.
    /// (Needs an installation; without one it says so and stops.)
    #[test]
    fn a_rear_section_comes_back_in_the_bend_it_was_in() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/Neoplan_N4421/Neoplan_N4421.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let ty = std::sync::Arc::new(omsi_sim::VehicleType::load(&root, &bus).expect("the bus loads"));
        let settled = || {
            let mut v = omsi_sim::VehicleInstance::new(ty.clone(), omsi_sim::VehicleHost::new(omsi_sim::SimClock::default()));
            crate::spawn::load_coupled_parts(&root, &mut v);
            for _ in 0..4 {
                v.update(1.0 / 30.0);
            }
            v
        };
        let mut v = settled();
        assert_eq!(v.trailers.len(), 1, "the N4421 is an articulated bus");
        let straight = v.trailers[0].heading;
        assert!((straight - v.heading).abs() < 1.0, "it stands straight behind the bus");
        // the axle pushed to the side, as a corner pushes it
        let at = v.trailers[0].pivot().expect("a frame has placed it") + glam::DVec3::new(4.0, 0.0, 0.0);
        v.trailers[0].set_pivot(at);
        v.update(1.0 / 30.0);
        let bent = v.trailers[0].heading;
        assert!((bent - straight).abs() > 10.0, "the rear section is in a bend now ({bent} against {straight})");

        // read again: without the axle it starts straight behind the bus, with it, bent
        let cold = settled();
        assert!((cold.trailers[0].heading - straight).abs() < 1.0, "a fresh one is straight");
        let mut kept = settled();
        kept.trailers[0].set_pivot(at);
        kept.update(1.0 / 30.0);
        assert!((kept.trailers[0].heading - bent).abs() < 0.5, "and one given the axle back is in the same bend");
    }

    /// What lies in the installation may not be written to; a mod or the content folder may.
    #[test]
    fn only_what_lies_outside_the_installation_can_be_edited() {
        let root = Path::new("/games/OMSI 2");
        assert!(in_installation(Path::new("/games/OMSI 2/Vehicles/MAN_SD200"), root));
        assert!(!in_installation(Path::new("/home/me/openOMSI/Vehicles/MAN_SD200"), root));
    }
}
