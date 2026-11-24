//! Independent local video-converter viewport.
use super::*;
use yttraydl::converter::{
    self, AudioBitrate, BatchEdit, BatchProgress, EncodingQuality, ResizePreset, Rotation,
    VideoProfile,
};
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
    quality: EncodingQuality,
    audio_bitrate: AudioBitrate,
    trim_start: String,
    trim_end: String,
    crop_width: String,
    crop_height: String,
    crop_x: String,
    crop_y: String,
    resize: ResizePreset,
    rotation: Rotation,
    mute_audio: bool,
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
        let mut batch_edit = None;
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
                        ui.separator();
                        if state.receiver.is_some() {
                            if state.status.is_empty() {
                                ui.label(
                                    "Converting… Closing this window will not stop the conversion.",
                                );
                            } else {
                                ui.label(&state.status);
                            }
                            ui.label("Closing this window does not stop the conversion.");
                            if ui.button("Cancel conversion").clicked() {
                                cancel = true;
                            }
                        } else if !state.status.is_empty() {
                            ui.label(&state.status);
                        }
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
                        if state.profile.is_audio_only() {
                            ui.label("Audio-only profiles ignore video options; FLAC is lossless.");
                        }
                        ui.add_enabled_ui(!state.profile.is_audio_only(), |ui| {
                        ui.label("Video encoding quality");
                        egui::ComboBox::from_id_salt("converter-quality")
                            .selected_text(state.quality.label())
                            .show_ui(ui, |ui| {
                                for quality in EncodingQuality::ALL {
                                    ui.selectable_value(&mut state.quality, quality, quality.label());
                                }
                            });
                        });
                        ui.label("Audio bitrate (lossy profiles)");
                        egui::ComboBox::from_id_salt("converter-audio-bitrate")
                            .selected_text(state.audio_bitrate.label())
                            .show_ui(ui, |ui| {
                                for bitrate in AudioBitrate::ALL {
                                    ui.selectable_value(&mut state.audio_bitrate, bitrate, bitrate.label());
                                }
                            });
                        ui.label("HEVC and AV1 need ffmpeg with the corresponding encoder; AV1 can be slow.");
                        ui.label("Optional trim (seconds; applies to single and batch)");
                        ui.horizontal(|ui| {
                            ui.label("Start");
                            ui.add(egui::TextEdit::singleline(&mut state.trim_start).desired_width(80.0).hint_text("0"));
                            ui.label("End");
                            ui.add(egui::TextEdit::singleline(&mut state.trim_end).desired_width(80.0).hint_text("end"));
                        });
                        ui.label("Up to 3 decimals. Empty end keeps the rest; output is re-encoded.");
                        ui.add_enabled_ui(!state.profile.is_audio_only(), |ui| {
                        ui.label("Optional crop (pixels before resize; applies to every file)");
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Width");
                            ui.add(egui::TextEdit::singleline(&mut state.crop_width).desired_width(62.0));
                            ui.label("Height");
                            ui.add(egui::TextEdit::singleline(&mut state.crop_height).desired_width(62.0));
                            ui.label("X");
                            ui.add(egui::TextEdit::singleline(&mut state.crop_x).desired_width(62.0).hint_text("0"));
                            ui.label("Y");
                            ui.add(egui::TextEdit::singleline(&mut state.crop_y).desired_width(62.0).hint_text("0"));
                        });
                        ui.label("Leave crop blank to keep the frame. Use even dimensions; ffmpeg may adjust edge offsets.");
                        ui.label("Rotation (after crop, before resize)");
                        egui::ComboBox::from_id_salt("converter-rotation")
                            .selected_text(state.rotation.label())
                            .show_ui(ui, |ui| {
                                for rotation in Rotation::ALL {
                                    ui.selectable_value(&mut state.rotation, rotation, rotation.label());
                                }
                            });
                        ui.checkbox(&mut state.mute_audio, "Remove audio from output");
                        ui.label("Resize (never upscale)");
                        egui::ComboBox::from_id_salt("converter-resize")
                            .selected_text(state.resize.label())
                            .show_ui(ui, |ui| {
                                for preset in ResizePreset::ALL {
                                    ui.selectable_value(&mut state.resize, preset, preset.label());
                                }
                            });
                        });
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
                        ui.add_enabled_ui(state.receiver.is_none(), |ui| ui.horizontal_wrapped(|ui| {
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
                        }));
                        ui.label(format!("{} files selected", state.batch_inputs.len()));
                        let count = state.batch_inputs.len();
                        let editable = state.receiver.is_none();
                        egui::ScrollArea::vertical().id_salt("converter-batch-list").max_height(180.0)
                            .show_rows(ui, 28.0, count, |ui, rows| {
                                for index in rows {
                                    ui.push_id(index, |ui| {
                                        ui.horizontal(|ui| {
                                            let path = state.batch_inputs[index].display().to_string();
                                            ui.add_sized([ (ui.available_width() - 190.0).max(20.0), 24.0 ], egui::Label::new(&path).truncate()).on_hover_text(&path);
                                            if ui.add_enabled(editable && index > 0, egui::Button::new("Up")).on_hover_text("Move this file earlier").clicked() {
                                                batch_edit = Some(BatchEdit::MoveUp(index));
                                            }
                                            if ui.add_enabled(editable && index + 1 < count, egui::Button::new("Down")).on_hover_text("Move this file later").clicked() {
                                                batch_edit = Some(BatchEdit::MoveDown(index));
                                            }
                                            if ui.add_enabled(editable, egui::Button::new("Remove")).on_hover_text("Remove this file from the batch").clicked() {
                                                batch_edit = Some(BatchEdit::Remove(index));
                                            }
                                        });
                                    });
                                }
                            });
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

                    });
                });
            },
        );
        self.show_converter = open;
        if let Some(edit) = batch_edit {
            converter::edit_batch(&mut state.batch_inputs, edit);
        }
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
            let quality = state.quality;
            let audio_bitrate = state.audio_bitrate;
            let trim_start = state.trim_start.clone();
            let trim_end = state.trim_end.clone();
            let crop_width = state.crop_width.clone();
            let crop_height = state.crop_height.clone();
            let crop_x = state.crop_x.clone();
            let crop_y = state.crop_y.clone();
            let resize = state.resize;
            let rotation = state.rotation;
            let mute_audio = state.mute_audio && !profile.is_audio_only();
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            let token = self.background.child_token();
            state.cancel = Some(token.clone());
            self.background.spawn(move |_| {
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()));
                let options =
                    converter::TrimRange::parse(&trim_start, &trim_end).and_then(|trim| {
                        let crop = if profile.is_audio_only() {
                            Ok(None)
                        } else {
                            converter::CropRect::parse(&crop_width, &crop_height, &crop_x, &crop_y)
                        };
                        crop.map(|crop| converter::ConversionOptions {
                            trim,
                            crop,
                            resize,
                            quality,
                            rotation,
                            mute_audio,
                            audio_bitrate,
                        })
                    });
                let specs = options.and_then(|options| {
                    converter::locate_ffmpeg(&settings, exe_dir.as_deref())
                        .ok_or_else(|| {
                            "ffmpeg not found. Configure or install it in Settings".to_string()
                        })
                        .and_then(|bin| {
                            if start_batch {
                                if batch_folder.trim().is_empty() {
                                    return Err("Choose a batch output folder".into());
                                }
                                converter::build_batch_with_options(
                                    &bin,
                                    &batch_inputs,
                                    std::path::Path::new(&batch_folder),
                                    profile,
                                    options,
                                )
                            } else {
                                converter::build_conversion_with_options(
                                    &bin,
                                    std::path::Path::new(&input),
                                    std::path::Path::new(&output),
                                    profile,
                                    options,
                                )
                                .map(|spec| vec![spec])
                            }
                        })
                });
                let result = specs.and_then(|specs| {
                    converter::run_batch_with_progress(&specs, &token, |event| {
                        let message = match event {
                            BatchProgress::Started { index, total } => {
                                format!("Converting file {index} of {total}…")
                            }
                            BatchProgress::EncodedTime {
                                index,
                                total,
                                microseconds,
                            } => {
                                let seconds = microseconds / 1_000_000;
                                format!(
                                    "File {index} of {total} · encoded {:02}:{:02}:{:02}",
                                    seconds / 3600,
                                    (seconds / 60) % 60,
                                    seconds % 60
                                )
                            }
                        };
                        let _ = tx.send(ConverterMessage::Progress(message));
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
