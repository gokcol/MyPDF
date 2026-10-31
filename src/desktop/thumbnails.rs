//! Follow reading navigation without preventing independent thumbnail browsing.
use super::*;

pub(super) const ROW_HEIGHT: f32 = 176.;

pub(super) fn scroll_area(
    ui: &egui::Ui,
    tab: &mut Tab,
    organizer: &mut organize::Organizer,
) -> egui::ScrollArea {
    let salt = (tab.id, "thumbnails");
    let area = egui::ScrollArea::vertical().id_salt(salt);
    if tab.pages.is_empty() || tab.thumbnail_page == Some(tab.page) {
        return area;
    }
    organizer.follow_reading_page(tab, tab.thumbnail_page);
    tab.thumbnail_page = Some(tab.page);

    // show_rows only creates visible thumbnails. Set the offset before it chooses
    // that range, so even Home/End or a distant page-number jump can be revealed.
    let id = ui.make_persistent_id(egui::Id::new(salt));
    let offset = egui::scroll_area::State::load(ui.ctx(), id).map_or(0., |state| state.offset.y);
    let top = tab.page as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y);
    let height = ui.available_height().max(0.);
    let bottom = top + ROW_HEIGHT.min(height);
    let offset = if top < offset {
        top
    } else if bottom > offset + height {
        bottom - height
    } else {
        offset
    };
    area.vertical_scroll_offset(offset.max(0.))
}
