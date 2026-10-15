#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[path = "../desktop/mod.rs"]
mod desktop;

fn main() -> eframe::Result {
    let paths = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    if desktop::open_with::hand_over(&paths) {
        return Ok(());
    }
    desktop::open_with::install();
    let viewport = eframe::egui::ViewportBuilder::default();
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false);
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("MyPDF")
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(eframe::egui::IconData {
                rgba: desktop::icon::rgba(128),
                width: 128,
                height: 128,
            })
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "MyPDF",
        options,
        Box::new(move |cc| Ok(Box::new(desktop::Viewer::new(cc, paths)))),
    )
}
