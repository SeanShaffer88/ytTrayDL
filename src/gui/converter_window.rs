//! Independent local video-converter viewport.
use super::*;
use yttraydl::converter::{self, VideoProfile};
use yttraydl::process::{self, ProcessLimits};

#[derive(Default)]
pub(super) struct ConverterState {
    input: String,
    output: String,
    profile: VideoProfile,
    status: String,
    receiver: Option<Receiver<Result<(), String>>>,
    cancel: Option<process::CancellationToken>,
}

impl App {
    pub(super) fn converter_ui(&mut self, ctx: &egui::Context) {
        let mut state = std::mem::take(&mut self.converter);
        if let Some(receiver) = &state.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    state.status = match result {
                        Ok(()) => "Conversion complete.".into(),
                        Err(error) => format!("Conversion failed: {error}. A partial output may remain."),
                    };
                    state.receiver = None;
                    state.cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    state.status = "Conversion worker stopped unexpectedly. A partial output may remain.".into();
                    state.receiver = None;
                    state.cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.show_converter {
            self.converter = state;
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut cancel = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-converter"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Video converter")
                .with_inner_size([620.0, 360.0])
                .with_min_inner_size([420.0, 300.0]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Video converter");
                    ui.label("Convert a local video without overwriting existing files.");
                    ui.add_space(8.0);
                    ui.label("Input file");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut state.input).desired_width((ui.available_width() - 90.0).max(40.0)));
                        if ui.button("Browse…").clicked() {
                            #[cfg(windows)]
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                state.input = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.label("Output profile");
                    egui::ComboBox::from_id_salt("converter-profile")
                        .selected_text(state.profile.label())
                        .show_ui(ui, |ui| {
                            for profile in VideoProfile::ALL {
                                ui.selectable_value(&mut state.profile, profile, profile.label());
                            }
                        });
                    ui.label("Output file (new file, with matching extension)");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut state.output).desired_width((ui.available_width() - 90.0).max(40.0)));
                        if ui.button("Browse…").clicked() {
                            #[cfg(windows)]
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter(state.profile.label(), &[state.profile.extension()])
                                .save_file()
                            {
                                state.output = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.add_space(8.0);
                    if ui.add_enabled(state.receiver.is_none(), egui::Button::new("Convert"))
                        .clicked()
                    {
                        start = true;
                    }
                    if state.receiver.is_some() {
                        if state.status.is_empty() {
                            ui.label("Converting… Closing this window will not stop the conversion.");
                        } else {
                            ui.label(&state.status);
                        }
                        if ui.button("Cancel conversion").clicked() {
                            cancel = true;
                        }
                    } else if !state.status.is_empty() {
                        ui.label(&state.status);
                    }
                    });
                });
            },
        );
        self.show_converter = open;
        if cancel {
            if let Some(token) = &state.cancel {
                token.cancel();
                state.status = "Cancelling…".into();
            }
        }
        if start {
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
            let spec = converter::locate_ffmpeg(&self.settings, exe_dir.as_deref())
                .ok_or_else(|| "ffmpeg not found. Configure or install it in Settings".to_string())
                .and_then(|bin| converter::build_conversion(&bin, std::path::Path::new(&state.input), std::path::Path::new(&state.output), state.profile));
            match spec {
                Err(error) => state.status = error,
                Ok(spec) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let wake = ctx.clone();
                    let token = self.background.child_token();
                    state.cancel = Some(token.clone());
                    self.background.spawn(move |_| {
                        let result = process::run_capture(&spec, ProcessLimits {
                            timeout: Duration::from_secs(6 * 60 * 60),
                            ..Default::default()
                        }, &token).map(|_| ()).map_err(|error| error.to_string());
                        let _ = tx.send(result);
                        wake.request_repaint();
                    });
                    state.status.clear();
                    state.receiver = Some(rx);
                }
            }
        }
        self.converter = state;
    }
}
