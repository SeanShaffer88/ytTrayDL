//! Independent, non-destructive audio tag and manual-lyrics editor.
use super::*;
use yttraydl::lyrics::{self, LyricsCandidate};
use yttraydl::tags::{self, AudioTags};

pub(super) enum TagResult {
    Loaded {
        path: String,
        result: Box<Result<AudioTags, String>>,
    },
    Search {
        input: String,
        title: String,
        artist: String,
        result: Result<Vec<LyricsCandidate>, String>,
    },
    Saved(Result<String, String>),
    ArtworkSaved(Result<String, String>),
}

#[derive(Default)]
pub(super) struct TagsState {
    input: String,
    output: String,
    artwork_output: String,
    replacement_artwork: String,
    loaded_input: Option<String>,
    tags: AudioTags,
    candidates: Vec<LyricsCandidate>,
    candidate_for: Option<(String, String, String)>,
    status: String,
    receiver: Option<Receiver<TagResult>>,
    cancel: Option<yttraydl::process::CancellationToken>,
}

impl App {
    pub(super) fn tags_ui(&mut self, ctx: &egui::Context) {
        let mut state = std::mem::take(&mut self.tags);
        if let Some(receiver) = &state.receiver {
            match receiver.try_recv() {
                Ok(TagResult::Loaded { path, result }) => {
                    if path != state.input {
                        state.loaded_input = None;
                        state.status = "Input changed; load it again.".into();
                    } else {
                        match *result {
                            Ok(tags) => {
                                state.output = tags::suggested_output(std::path::Path::new(&path))
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                state.artwork_output = tags
                                    .artwork
                                    .as_ref()
                                    .and_then(|artwork| {
                                        let path = std::path::Path::new(&path);
                                        let mut name = path.file_stem()?.to_os_string();
                                        name.push(".cover.");
                                        name.push(artwork.extension);
                                        Some(
                                            path.with_file_name(name)
                                                .to_string_lossy()
                                                .into_owned(),
                                        )
                                    })
                                    .unwrap_or_default();
                                state.loaded_input = Some(path);
                                state.tags = tags;
                                state.candidates.clear();
                                state.candidate_for = None;
                                state.replacement_artwork.clear();
                                state.status = "Tags loaded. Export creates a new copy.".into();
                            }
                            Err(error) => {
                                state.loaded_input = None;
                                state.status = format!("Could not read tags: {error}");
                            }
                        }
                    }
                    state.receiver = None;
                    state.cancel = None;
                }
                Ok(TagResult::Search {
                    input,
                    title,
                    artist,
                    result,
                }) => {
                    state.candidates.clear();
                    state.candidate_for = None;
                    if input != state.input
                        || title != state.tags.title
                        || artist != state.tags.artist
                    {
                        state.status = "Input or search terms changed; search again.".into();
                    } else {
                        match result {
                            Ok(candidates) => {
                                state.status = if candidates.is_empty() {
                                    "No plain-lyrics results found.".into()
                                } else {
                                    "Review results and choose lyrics to use.".into()
                                };
                                state.candidate_for = Some((input, title, artist));
                                state.candidates = candidates;
                            }
                            Err(error) => state.status = error,
                        }
                    }
                    state.receiver = None;
                    state.cancel = None;
                }
                Ok(TagResult::Saved(result) | TagResult::ArtworkSaved(result)) => {
                    state.status = match result {
                        Ok(message) => message,
                        Err(error) => {
                            format!("Export failed: {error}. A partial output may remain.")
                        }
                    };
                    state.receiver = None;
                    state.cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    state.status =
                        "Tag worker stopped unexpectedly; a partial output may remain.".into();
                    state.receiver = None;
                    state.cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.show_tags {
            self.tags = state;
            return;
        }
        let mut open = true;
        let mut load = false;
        let mut save = false;
        let mut export_artwork = false;
        let mut search = false;
        let mut chosen_lyrics = None;
        let mut cancel = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-tags"),
            egui::ViewportBuilder::default()
                .with_title("ytTrayDL — Lyrics & tags")
                .with_inner_size([620.0, 560.0])
                .with_min_inner_size([420.0, 360.0]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) { open = false; }
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Lyrics & tags");
                        ui.label("Edit a tagged copy of a local audio file. The original is not changed.");
                        if !state.status.is_empty() { ui.label(&state.status); }
                        if state.receiver.is_some() && ui.button("Cancel").clicked() { cancel = true; }
                        ui.label("Input audio (MP3, M4A, FLAC, OGG or Opus)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut state.input).desired_width((ui.available_width() - 90.0).max(40.0)));
                            if ui.button("Browse…").clicked() {
                                #[cfg(windows)]
                                if let Some(path) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "m4a", "flac", "ogg", "opus"]).pick_file() {
                                    state.input = path.to_string_lossy().into_owned();
                                }
                            }
                        });
                        if ui.add_enabled(state.receiver.is_none(), egui::Button::new("Load tags")).clicked() { load = true; }
                        let loaded = state.loaded_input.as_deref() == Some(state.input.as_str()) && state.receiver.is_none();
                        ui.add_enabled_ui(loaded, |ui| {
                            ui.label("Title"); ui.text_edit_singleline(&mut state.tags.title);
                            ui.label("Artist"); ui.text_edit_singleline(&mut state.tags.artist);
                            ui.label("Album"); ui.text_edit_singleline(&mut state.tags.album);
                            ui.label("Album artist"); ui.text_edit_singleline(&mut state.tags.album_artist);
                            ui.label("Track number"); ui.text_edit_singleline(&mut state.tags.track);
                            ui.label("Date"); ui.text_edit_singleline(&mut state.tags.date);
                            ui.label("Genre"); ui.text_edit_singleline(&mut state.tags.genre);
                            ui.label("Lyrics");
                            if state.tags.lyrics_present {
                                ui.label("Existing lyrics detected; they will not be replaced.");
                            }
                            ui.add_enabled(!state.tags.lyrics_present, egui::TextEdit::multiline(&mut state.tags.lyrics).desired_rows(6).desired_width(ui.available_width()));
                            ui.label("Online search sends title and artist to LRCLIB when you choose Search. Results may be inaccurate; review before export.");
                            if ui.add_enabled(!state.tags.lyrics_present, egui::Button::new("Search online for lyrics")).clicked() { search = true; }
                            if !state.candidates.is_empty() {
                                ui.label("Search results (choose one to fill empty lyrics):");
                                egui::ScrollArea::vertical().id_salt("lyrics-search-results").max_height(150.0).show_rows(ui, 28.0, state.candidates.len(), |ui, rows| {
                                    for index in rows {
                                        let item = &state.candidates[index];
                                        ui.horizontal(|ui| {
                                            let name = format!("{} — {} ({})", item.artist, item.title, item.album);
                                            ui.add_sized([(ui.available_width() - 72.0).max(20.0), 24.0], egui::Label::new(&name).truncate()).on_hover_text(&name);
                                            if ui.add_enabled(!state.tags.lyrics_present && state.tags.lyrics.is_empty(), egui::Button::new("Use")).clicked() { chosen_lyrics = Some(index); }
                                        });
                                    }
                                });
                                if !state.tags.lyrics.is_empty() { ui.label("Clear the lyrics field before choosing a result."); }
                            }
                            ui.label("Output copy (same extension; must not exist)");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut state.output).desired_width((ui.available_width() - 90.0).max(40.0)));
                                if ui.button("Browse…").clicked() {
                                    #[cfg(windows)]
                                    if let Some(path) = rfd::FileDialog::new().save_file() {
                                        state.output = path.to_string_lossy().into_owned();
                                    }
                                }
                            });
                            ui.label("Optional replacement cover (JPEG/PNG, up to 10 MiB; MP3/M4A/FLAC only)");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut state.replacement_artwork).desired_width((ui.available_width() - 90.0).max(40.0)).hint_text("Leave blank to keep existing artwork"));
                                if ui.button("Browse…").clicked() {
                                    #[cfg(windows)]
                                    if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png"]).pick_file() {
                                        state.replacement_artwork = path.to_string_lossy().into_owned();
                                    }
                                }
                            });
                            ui.label("A selected cover replaces all detected embedded covers in the copy, not the original. Other tags or artwork may be changed or omitted by ffmpeg; check the copy before relying on it.");
                            if ui.button("Export tagged copy").clicked() { save = true; }
                            ui.separator();
                            ui.heading("Embedded artwork");
                            if let Some(artwork) = &state.tags.artwork {
                                ui.label(format!("Embedded {} artwork detected. Export the original image to a new file.", artwork.extension.to_uppercase()));
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut state.artwork_output).desired_width((ui.available_width() - 90.0).max(40.0)));
                                    if ui.button("Browse…").clicked() {
                                        #[cfg(windows)]
                                        if let Some(path) = rfd::FileDialog::new().add_filter("Image", &[artwork.extension]).save_file() {
                                            state.artwork_output = path.to_string_lossy().into_owned();
                                        }
                                    }
                                });
                                if ui.button("Export artwork").clicked() { export_artwork = true; }
                            } else {
                                ui.label("No supported embedded JPEG or PNG artwork detected.");
                            }
                        });
                    });
                });
            },
        );
        self.show_tags = open;
        let search_matches = state
            .candidate_for
            .as_ref()
            .is_some_and(|(input, title, artist)| {
                input == &state.input && title == &state.tags.title && artist == &state.tags.artist
            });
        if !search_matches {
            state.candidates.clear();
            state.candidate_for = None;
        }
        if let Some(candidate) = chosen_lyrics
            .filter(|_| {
                search_matches && !state.tags.lyrics_present && state.tags.lyrics.is_empty()
            })
            .and_then(|index| state.candidates.get(index))
        {
            state.tags.lyrics = candidate.lyrics.clone();
            state.status = "Lyrics copied into the editor. Review before exporting.".into();
        }
        if let (true, Some(token)) = (cancel, &state.cancel) {
            token.cancel();
            state.status = "Cancelling…".into();
        }
        if load || save || export_artwork || search {
            let input = state.input.clone();
            let output = if export_artwork {
                state.artwork_output.clone()
            } else {
                state.output.clone()
            };
            let edits = state.tags.clone();
            let replacement_artwork = state.replacement_artwork.clone();
            let settings = self.settings.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            let token = self.background.child_token();
            state.cancel = Some(token.clone());
            self.background.spawn(move |_| {
                if search {
                    let title = edits.title.clone();
                    let artist = edits.artist.clone();
                    let result = lyrics::search(&title, &artist, &token);
                    let _ = tx.send(TagResult::Search { input, title, artist, result });
                    wake.request_repaint();
                    return;
                }
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()));
                let result = (|| {
                    let ffmpeg = yttraydl::converter::locate_ffmpeg(&settings, exe_dir.as_deref())
                        .ok_or("ffmpeg not found; configure or install it in Settings")?;
                    let ffprobe = tags::locate_ffprobe(&ffmpeg)
                        .ok_or("ffprobe not found; install it alongside ffmpeg or on PATH")?;
                    let existing = tags::inspect(&ffprobe, std::path::Path::new(&input), &token)?;
                    if load {
                        return Ok(existing);
                    }
                    let spec = if export_artwork {
                        let artwork = existing.artwork.as_ref().ok_or("No supported embedded artwork found")?;
                        tags::build_artwork_export(&ffmpeg, std::path::Path::new(&input), std::path::Path::new(&output), artwork)?
                    } else {
                        let replacement = if replacement_artwork.trim().is_empty() { None } else { Some(std::path::Path::new(&replacement_artwork)) };
                        tags::build_tagged_copy_with_artwork(&ffmpeg, std::path::Path::new(&input), std::path::Path::new(&output), &edits, &existing, replacement)?
                    };
                    yttraydl::process::run_capture(
                        &spec,
                        yttraydl::process::ProcessLimits {
                            timeout: Duration::from_secs(6 * 60 * 60),
                            ..Default::default()
                        },
                        &token,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(existing)
                })();
                let message = if load {
                    TagResult::Loaded {
                        path: input,
                        result: Box::new(result),
                    }
                } else if export_artwork {
                    TagResult::ArtworkSaved(result.map(|_| "Artwork exported to a new image file.".into()))
                } else {
                    TagResult::Saved(result.map(|existing| {
                        if existing.lyrics_present {
                            "Tagged copy exported without replacing detected lyrics. Check its tags and artwork.".into()
                        } else {
                            "Tagged copy exported. Check its tags and artwork.".into()
                        }
                    }))
                };
                let _ = tx.send(message);
                wake.request_repaint();
            });
            if search {
                state.candidates.clear();
                state.candidate_for = None;
            }
            state.status = if search {
                "Searching online…"
            } else if load {
                "Reading tags…"
            } else if export_artwork {
                "Exporting artwork…"
            } else {
                "Exporting copy…"
            }
            .into();
            state.receiver = Some(rx);
        }
        self.tags = state;
    }
}
