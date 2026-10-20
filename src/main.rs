// Release builds are a tray app: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gui;

use yttraydl::platform;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let show = platform::launched_with_show_flag(&args);

    // Manual URL argument: `yttraydl.exe https://…` acts like copying
    // the link (handy for browser "send to" integrations and testing).
    let manual_url = args
        .iter()
        .skip(1)
        .find(|a| a.starts_with("https://") || a.starts_with("http://"))
        .cloned();

    let setup = platform::setup(show);
    if setup.already_running {
        // The running instance has been told to show itself.
        return Ok(());
    }

    let icon = gui::app_icon_data();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ytTrayDL")
            .with_inner_size([400.0, 540.0])
            .with_min_inner_size([340.0, 420.0])
            .with_visible(false) // tray-first: the panel opens on demand
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "ytTrayDL",
        options,
        Box::new(move |cc| Ok(Box::new(gui::App::new(cc, setup, show, manual_url.clone())))),
    )
}
