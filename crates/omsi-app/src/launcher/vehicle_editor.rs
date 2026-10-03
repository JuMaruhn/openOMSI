//! The Vehicle Editor page: the chosen bus by itself, in the picture the game's own renderer
//! draws of it (see `showroom`), over the whole page. The mouse turns it and the wheel zooms
//! it, as on the Drive page's card - `Launcher::preview` does both, and the picture is the
//! one the showroom has already made for that page, so opening this one costs nothing.
//! Nothing else is drawn yet: the controls that will change a vehicle come over this picture.

use super::Launcher;
use omsi_ui::Rect;

/// The page: the showroom over the whole of `r`.
///
/// A phone lays its pages out taller than its screen and scrolls them (`mobile::PAGE_H`), so
/// the picture is held to what the screen shows instead of growing with the page - a preview
/// taller than the window would be rendered at that size and mostly never seen.
pub fn draw(l: &mut Launcher, r: Rect) {
    let h = r.h.min(l.ui.size.y - r.y - 24.0).max(200.0);
    l.preview(Rect::new(r.x, r.y, r.w, h));
}
