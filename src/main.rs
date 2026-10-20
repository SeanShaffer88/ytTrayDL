// Release builds are a tray app: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gui;

use yttraydl::platform;

fn main() -> eframe::Result<()> {
    // No console in a tray app: panics go to the local diagnostic
    // log so a silent death is always explainable after the fact.
    std::panic::set_hook(Box::new(|info| {
        yttraydl::diag_log(&format!("PANIC: {info}"));
    }));

    let args: Vec<String> = std::env::args().collect();
    let show = platform::launched_with_show_flag(&args);
    // `--background` is how Start-with-Windows launches the app:
    // straight to the tray, no window. A manual launch (double-click,
    // Start menu) opens the control panel instead — starting
    // completely invisibly made the first release look broken.
    let background = args.iter().any(|a| a == "--background");

    // Manual URL argument: `yttraydl.exe https://…` acts like copying
    // the link (handy for browser "send to" integrations and testing).
    let manual_url = args
        .iter()
        .skip(1)
        .find(|a| a.starts_with("https://") || a.starts_with("http://"))
        .cloned();

    let setup = platform::setup(show);
    yttraydl::diag_log(&format!(
        "launch: background={background} show={show} already_running={}",
        setup.already_running
    ));
    if setup.already_running {
        // The running instance has been told to show itself.
        return Ok(());
    }

    let open_panel = show || !background;
    let icon = gui::app_icon_data();
    let options = eframe::NativeOptions {
        // Glow (OpenGL), pinned explicitly — see Cargo.toml for
        // why wgpu is not used: the 0.1.0/0.1.1 builds enabled
        // wgpu with no backend feature, which panicked at startup
        // on a real Windows PC. Stating the renderer here keeps
        // the choice pinned even if crate features change.
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("ytTrayDL")
            .with_inner_size([400.0, 540.0])
            .with_min_inner_size([340.0, 420.0])
            .with_visible(false) // tray-first: the panel opens on demand
            .with_icon(icon),
        ..Default::default()
    };

    // catch_unwind: a panic inside UI startup (like the wgpu one)
    // must surface as an error dialog + log line, never a silent
    // exit. The panic hook above logs it too; this makes it seen.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            "ytTrayDL",
            options,
            Box::new(move |cc| {
                Ok(Box::new(gui::App::new(
                    cc,
                    setup,
                    open_panel,
                    manual_url.clone(),
                )))
            }),
        )
    }));
    let log_path = yttraydl::settings::config_dir().join("yttraydl.log");
    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            yttraydl::diag_log(&format!("fatal: UI failed to start: {e}"));
            platform::fatal_error_box(
                "ytTrayDL could not start",
                &format!(
                    "The window could not be created ({e}).\n\nDetails were written to the log:\n{}",
                    log_path.display()
                ),
            );
            Err(e)
        }
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            yttraydl::diag_log(&format!("fatal: startup panic: {msg}"));
            platform::fatal_error_box(
                "ytTrayDL could not start",
                &format!(
                    "The app crashed while starting ({msg}).\n\nDetails were written to the log:\n{}",
                    log_path.display()
                ),
            );
            Ok(())
        }
    }
}
