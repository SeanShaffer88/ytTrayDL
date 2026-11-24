//! Independent local video-converter viewport.
use super::*;
use yttraydl::converter::{self, VideoProfile};
use yttraydl::process;

enum ConverterMessage {
    Progress(String),
    Finished(Result<(), String>),
}

#[derive(Default)]
pub(super) struct ConverterState {
    input: String,
    output: String,
    profile: VideoProfile,
    trim_start: String,
    trim_end: String,
    status: String,
    batch_inputs: Vec<std::path::PathBuf>,
    batch_folder: String,
    receiver: Option<Receiver<ConverterMessage>>,
    cancel: Option<process::CancellationToken>,
}

impl App {
    pub(super) fn converter_ui(&mut self, ctx: &egui::Context) {
        let mut state = std::mem::take(&mut self.converter);
        if let Some(receiver) = &state.receiver {
            let mut finished = false;
            loop {
                match receiver.try_recv() {
                    Ok(ConverterMessage::Progress(message)) => state.status = message,
                    Ok(ConverterMessage::Finished(result)) => {
                        state.status = match result {
                            Ok(()) => "Conversion complete.".into(),
                            Err(error) => {
                                format!("Conversion stopped: {error}. A partial output may remain.")
                            }
                        };
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        state.status =
                            "Conversion worker stopped unexpectedly. A partial output may remain."
                                .into();
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                }
            }
            if finished {
                state.receiver = None;
                state.cancel = None;
            }
        }
        if !self.show_converter {
            self.converter = state;
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut start_batch = false;
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
                            ui.add(
                                egui::TextEdit::singleline(&mut state.input)
                                    .desired_width((ui.available_width() - 90.0).max(40.0)),
                            );
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
                                    ui.selectable_value(
                                        &mut state.profile,
                                        profile,
                                        profile.label(),
                                    );
                                }
                            });
                        ui.label("Optional trim (seconds; applies to single and batch)");
                        ui.horizontal(|ui| {
                            ui.label("Start");
                            ui.add(egui::TextEdit::singleline(&mut state.trim_start).desired_width(80.0).hint_text("0"));
                            ui.label("End");
                            ui.add(egui::TextEdit::singleline(&mut state.trim_end).desired_width(80.0).hint_text("end"));
                        });
                        ui.label("Up to 3 decimals. Empty end keeps the rest; output is re-encoded.");
                        ui.label("Output file (new file, with matching extension)");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.output)
                                    .desired_width((ui.available_width() - 90.0).max(40.0)),
                            );
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
                        if ui
                            .add_enabled(state.receiver.is_none(), egui::Button::new("Convert"))
                            .clicked()
                        {
                            start = true;
                        }
                        ui.separator();
                        ui.heading("Batch conversion");
                        ui.label("Convert selected files one at a time into an existing folder.");
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Add input above to batch").clicked() && !state.input.trim().is_empty() {
                                state.batch_inputs.push(state.input.trim().into());
                            }
                            if ui.button("Choose files…").clicked() {
                                #[cfg(windows)]
                                if let Some(files) = rfd::FileDialog::new().pick_files() {
                                    state.batch_inputs.extend(files);
                                }
                            }
                            if ui.button("Clear batch").clicked() {
                                state.batch_inputs.clear();
                            }
                        });
                        ui.label(format!("{} files selected", state.batch_inputs.len()));
                        for path in state.batch_inputs.iter().take(5) {
                            ui.add(egui::Label::new(path.display().to_string()).truncate());
                        }
                        if state.batch_inputs.len() > 5 {
                            ui.label(format!("and {} more", state.batch_inputs.len() - 5));
                        }
                        ui.label("Batch output folder");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut state.batch_folder).desired_width((ui.available_width() - 90.0).max(40.0)));
                            if ui.button("Browse…").clicked() {
                                if let Some(path) = pick_folder(std::path::Path::new(&state.batch_folder)) {
                                    state.batch_folder = path.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.label("Outputs use the input name plus .converted and the selected profile extension.");
                        if ui.add_enabled(state.receiver.is_none(), egui::Button::new("Convert batch")).clicked() {
                            start_batch = true;
                        }
                        ui.separator();
                        if state.receiver.is_some() {
                            if state.status.is_empty() {
                                ui.label(
                                    "Converting… Closing this window will not stop the conversion.",
                                );
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
        if let (true, Some(token)) = (cancel, &state.cancel) {
            token.cancel();
            state.status = "Cancelling…".into();
        }
        if start || start_batch {
            let settings = self.settings.clone();
            let input = state.input.clone();
            let output = state.output.clone();
            let batch_inputs = state.batch_inputs.clone();
            let batch_folder = state.batch_folder.clone();
            let profile = state.profile;
            let trim_start = state.trim_start.clone();
            let trim_end = state.trim_end.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            let token = self.background.child_token();
            state.cancel = Some(token.clone());
            self.background.spawn(move |_| {
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()));
                let specs = converter::TrimRange::parse(&trim_start, &trim_end).and_then(|trim| {
                    converter::locate_ffmpeg(&settings, exe_dir.as_deref())
                        .ok_or_else(|| {
                            "ffmpeg not found. Configure or install it in Settings".to_string()
                        })
                        .and_then(|bin| {
                            if start_batch {
                                if batch_folder.trim().is_empty() {
                                    return Err("Choose a batch output folder".into());
                                }
                                converter::build_batch_with_trim(
                                    &bin,
                                    &batch_inputs,
                                    std::path::Path::new(&batch_folder),
                                    profile,
                                    trim,
                                )
                            } else {
                                converter::build_conversion_with_trim(
                                    &bin,
                                    std::path::Path::new(&input),
                                    std::path::Path::new(&output),
                                    profile,
                                    trim,
                                )
                                .map(|spec| vec![spec])
                            }
                        })
                });
                let result = specs.and_then(|specs| {
                    converter::run_batch(&specs, &token, |index, total| {
                        let _ = tx.send(ConverterMessage::Progress(format!(
                            "Converting file {index} of {total}…"
                        )));
                        wake.request_repaint();
                    })
                });
                let _ = tx.send(ConverterMessage::Finished(result));
                wake.request_repaint();
            });
            state.status = "Preparing conversion…".into();
            state.receiver = Some(rx);
        }
        self.converter = state;
    }
}
