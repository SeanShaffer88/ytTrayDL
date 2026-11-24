//! Local playlist manager; ffplay provides the playback window and transport keys.
use super::*;
use yttraydl::player::{self, PlayQueue};
use yttraydl::process::CancellationToken;

struct PlayerResult {
    index: usize,
    result: Result<(), String>,
}

enum QueueIoResult {
    Saved(Result<(), String>),
    Loaded(Result<PlayQueue, String>),
    Deleted(Result<(), String>),
    Scanned(Result<player::FolderScan, String>),
    OptionsSaved(Result<(), String>),
    OptionsLoaded(Result<player::PlayerOptions, String>),
}

#[derive(Clone, Copy)]
enum QueueIoAction {
    Save,
    Load,
    Delete,
    SaveOptions,
    LoadOptions,
}

pub(super) struct PlayerState {
    queue: PlayQueue,
    input: String,
    filter: String,
    ffplay_path: String,
    selected: usize,
    active: Option<usize>,
    volume: u8,
    repeat: bool,
    pending: Option<usize>,
    stopping: bool,
    status: String,
    receiver: Option<Receiver<PlayerResult>>,
    cancel: Option<CancellationToken>,
    queue_io: Option<Receiver<QueueIoResult>>,
    scan_cancel: Option<CancellationToken>,
    scan_cancel_requested: bool,
}
impl Default for PlayerState {
    fn default() -> Self {
        Self {
            queue: PlayQueue::default(),
            input: String::new(),
            filter: String::new(),
            ffplay_path: String::new(),
            selected: 0,
            active: None,
            volume: 75,
            repeat: false,
            pending: None,
            stopping: false,
            status: String::new(),
            receiver: None,
            cancel: None,
            queue_io: None,
            scan_cancel: None,
            scan_cancel_requested: false,
        }
    }
}

impl App {
    fn start_player_track(&self, state: &mut PlayerState, index: usize, ctx: &egui::Context) {
        let Some(input) = state.queue.items().get(index).cloned() else {
            return;
        };
        let explicit = state.ffplay_path.clone();
        let settings = self.settings.clone();
        let volume = state.volume;
        let token = self.background.child_token();
        let worker_token = token.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        state.active = Some(index);
        state.selected = index;
        state.stopping = false;
        state.pending = None;
        state.status = format!(
            "Playing {} of {}: {}",
            index + 1,
            state.queue.items().len(),
            input.display()
        );
        state.cancel = Some(token);
        state.receiver = Some(rx);
        self.background.spawn(move |_| {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()));
            let ffmpeg = yttraydl::converter::locate_ffmpeg(&settings, exe_dir.as_deref());
            let explicit = if explicit.trim().is_empty() {
                None
            } else {
                Some(std::path::Path::new(&explicit))
            };
            let result = player::locate_ffplay(explicit, ffmpeg.as_deref())
                .ok_or_else(|| {
                    "ffplay not found. Select ffplay or install it next to ffmpeg or on PATH"
                        .to_string()
                })
                .and_then(|bin| player::build_playback(&bin, &input, volume))
                .and_then(|spec| player::play(&spec, &worker_token));
            let _ = tx.send(PlayerResult { index, result });
            wake.request_repaint();
        });
    }

    pub(super) fn player_ui(&mut self, ctx: &egui::Context) {
        let mut state = std::mem::take(&mut self.player);
        if let Some(receiver) = &state.queue_io {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(QueueIoResult::Saved(
                    Err("Queue storage worker stopped unexpectedly".into()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                state.queue_io = None;
                state.scan_cancel = None;
                let scan_cancelled = std::mem::take(&mut state.scan_cancel_requested);
                state.status = match result {
                    QueueIoResult::Saved(Ok(())) => "Queue saved to the app data folder.".into(),
                    QueueIoResult::Loaded(Ok(queue)) => {
                        let count = queue.items().len();
                        let added = state.queue.add(queue.items().iter().cloned());
                        format!(
                            "Appended {added} of {count} saved file(s); queue limit is {}.",
                            player::MAX_QUEUE
                        )
                    }
                    QueueIoResult::Deleted(Ok(())) => {
                        "Saved queue deleted; the current queue is unchanged.".into()
                    }
                    QueueIoResult::Scanned(_) if scan_cancelled => {
                        "Folder scan cancelled; no files added.".into()
                    }
                    QueueIoResult::Scanned(Ok(scan)) => {
                        let added = state.queue.add(scan.files);
                        let limit = if scan.limit_reached {
                            " Scan or queue limit reached."
                        } else {
                            ""
                        };
                        format!("Added {added} file(s). Skipped {} links or inaccessible entries.{limit}", scan.skipped)
                    }
                    QueueIoResult::Scanned(Err(error)) => format!("Folder scan failed: {error}"),
                    QueueIoResult::OptionsSaved(Ok(())) => {
                        "Player options saved to app data.".into()
                    }
                    QueueIoResult::OptionsLoaded(Ok(options)) => {
                        state.ffplay_path = options.ffplay_path;
                        state.volume = options.volume;
                        state.repeat = options.repeat;
                        "Player options restored; playback has not started.".into()
                    }
                    QueueIoResult::OptionsSaved(Err(error))
                    | QueueIoResult::OptionsLoaded(Err(error)) => {
                        format!("Player options failed: {error}")
                    }
                    QueueIoResult::Saved(Err(error))
                    | QueueIoResult::Loaded(Err(error))
                    | QueueIoResult::Deleted(Err(error)) => {
                        format!("Queue storage failed: {error}")
                    }
                };
            }
        }
        let mut start_after = None;
        if let Some(rx) = &state.receiver {
            let event = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(PlayerResult {
                    index: state.active.unwrap_or(0),
                    result: Err("Playback worker stopped unexpectedly".into()),
                }),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(event) = event {
                state.receiver = None;
                state.cancel = None;
                state.active = None;
                match player::completion(
                    &state.queue,
                    event.index,
                    state.repeat,
                    state.stopping,
                    state.pending.take(),
                    event.result.is_ok(),
                ) {
                    player::Completion::Start(next) => start_after = Some(next),
                    player::Completion::Stopped => state.status = "Playback stopped.".into(),
                    player::Completion::Finished => state.status = "Queue finished.".into(),
                    player::Completion::Failed => {
                        state.status = format!("Playback stopped: {}", event.result.unwrap_err())
                    }
                }
                state.stopping = false;
            }
        }
        if let Some(index) = start_after {
            self.start_player_track(&mut state, index, ctx);
        }
        if !self.show_player {
            self.player = state;
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut stop = false;
        let mut skip_to = None;
        let mut remove = None;
        let mut up = None;
        let mut down = None;
        let mut queue_action = None;
        let mut scan_folder = None;
        let mut cancel_scan = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("yttraydl-player"),
            egui::ViewportBuilder::default().with_title("ytTrayDL — Player")
                .with_inner_size([640.0, 520.0]).with_min_inner_size([420.0, 340.0]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) { open = false; }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("Local player");
                    ui.label("Play local audio and video with a separately installed ffplay.");
                    if !state.status.is_empty() { ui.add(egui::Label::new(&state.status).truncate()).on_hover_text(&state.status); }
                    let playing = state.receiver.is_some();
                    let busy = playing || state.queue_io.is_some();
                    ui.horizontal(|ui| {
                        ui.label("Filter queue");
                        ui.add(egui::TextEdit::singleline(&mut state.filter).char_limit(256).hint_text("Search file paths"));
                        if ui.button("Clear filter").clicked() { state.filter.clear(); }
                    });
                    let visible = state.queue.matching_indices(&state.filter);
                    let selected_visible = visible.contains(&state.selected);
                    if !state.queue.items().is_empty() && !selected_visible {
                        ui.label("Selected track is hidden; select a visible track or clear the filter to play it.");
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!busy && !state.queue.items().is_empty() && selected_visible, egui::Button::new("Play selected")).clicked() { start = true; }
                        if ui.add_enabled(playing && !state.stopping, egui::Button::new("Stop")).clicked() { stop = true; }
                        let prev = state.active.and_then(|i| i.checked_sub(1));
                        if ui.add_enabled(playing && !state.stopping && prev.is_some(), egui::Button::new("Previous")).clicked() { skip_to = prev; }
                        let next = state.active.and_then(|i| state.queue.next(i, state.repeat));
                        if ui.add_enabled(playing && !state.stopping && next.is_some(), egui::Button::new("Next")).clicked() { skip_to = next; }
                        ui.add_enabled(state.queue_io.is_none(), egui::Checkbox::new(&mut state.repeat, "Repeat queue"));
                    });
                    if state.scan_cancel.is_some() && ui.button("Cancel folder scan").clicked() { cancel_scan = true; }
                    ui.label("Playback window: Space = pause, arrows = seek, 9/0 = volume, M = mute.")
                        .on_hover_text("Closing the playback window advances the queue. Closing this manager leaves playback running.");
                    ui.separator();
                    egui::CollapsingHeader::new("Media & playback options").default_open(false).show(ui, |ui| {
                    let max_options_height = (ui.available_height() - 90.0).clamp(60.0, 220.0);
                    egui::ScrollArea::vertical().id_salt("player-options").max_height(max_options_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.label("ffplay executable (optional; not included in the managed ffmpeg download)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut state.ffplay_path).desired_width((ui.available_width() - 90.0).max(40.0)).hint_text("Auto-detect next to ffmpeg or on PATH"));
                            if ui.button("Browse…").clicked() {
                                #[cfg(windows)]
                                if let Some(path) = rfd::FileDialog::new().add_filter("Application", &["exe"]).pick_file() { state.ffplay_path = path.to_string_lossy().into_owned(); }
                            }
                        });
                        ui.add(egui::Slider::new(&mut state.volume, 0..=100).text("Starting volume"));
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Save player options").clicked() { queue_action = Some(QueueIoAction::SaveOptions); }
                            if ui.button("Restore player options").clicked() { queue_action = Some(QueueIoAction::LoadOptions); }
                        });
                        ui.label("Options are restored only on request; playback never starts automatically.");
                        ui.label("Add a local audio or video file");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut state.input).desired_width((ui.available_width() - 140.0).max(40.0)));
                            if ui.button("Add").clicked() {
                                let added = state.queue.add([std::path::PathBuf::from(state.input.trim())]);
                                state.status = if added == 1 { "Added to queue." } else { "Unsupported file type or queue is full." }.into();
                            }
                            if ui.button("Choose…").clicked() {
                                #[cfg(windows)]
                                if let Some(paths) = rfd::FileDialog::new().pick_files() {
                                    let added = state.queue.add(paths);
                                    state.status = format!("Added {added} file(s). Unsupported types and entries beyond the limit were skipped.");
                                }
                            }
                        });
                        if ui.add_enabled(state.queue.items().len() < player::MAX_QUEUE, egui::Button::new("Add folder…")).clicked() {
                            scan_folder = pick_folder(std::path::Path::new(&state.input));
                        }
                        ui.label("Folder scan includes subfolders, skips links, and stops at safety or queue limits.");
                        if ui.button("Clear queue").clicked() { state.queue.clear(); state.selected = 0; }
                        ui.label("Saved queue (optional; stores file paths in app data)");
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!state.queue.items().is_empty(), egui::Button::new("Save / replace saved queue")).clicked() { queue_action = Some(QueueIoAction::Save); }
                            if ui.button("Append saved queue").clicked() { queue_action = Some(QueueIoAction::Load); }
                            if ui.button("Delete saved queue").clicked() { queue_action = Some(QueueIoAction::Delete); }
                        });
                    });
                    });
                    });
                    ui.separator();
                    ui.label(format!("Queue: {} / {} · showing {}", state.queue.items().len(), player::MAX_QUEUE, visible.len()));
                    if !state.filter.trim().is_empty() {
                        ui.label("Filter only changes the view. Playback and Up/Down use the full queue.");
                    }
                    let count = state.queue.items().len();
                    egui::ScrollArea::vertical().id_salt("player-queue").auto_shrink([false, false]).show_rows(ui, 30.0, visible.len(), |ui, rows| {
                        for row in rows {
                            let index = visible[row];
                            ui.push_id(index, |ui| ui.horizontal(|ui| {
                                let path = state.queue.items()[index].display().to_string();
                                let available = (ui.available_width() - 175.0).max(20.0);
                                if ui.add_enabled_ui(!busy, |ui| ui.add_sized([available, 24.0], egui::Button::new(&path).selected(state.selected == index))).inner.on_hover_text(&path).clicked() { state.selected = index; }
                                if ui.add_enabled(!busy && index > 0, egui::Button::new("Up")).clicked() { up = Some(index); }
                                if ui.add_enabled(!busy && index + 1 < count, egui::Button::new("Down")).clicked() { down = Some(index); }
                                if ui.add_enabled(!busy, egui::Button::new("Remove")).clicked() { remove = Some(index); }
                            }));
                        }
                    });
                });
            },
        );
        self.show_player = open;
        if let Some(index) = remove {
            state.queue.remove(index);
            state.selected =
                player::selection_after_remove(state.selected, index, state.queue.items().len());
        }
        if let Some(index) = up.filter(|&index| state.queue.move_up(index)) {
            state.selected = index - 1;
        }
        if let Some(index) = down.filter(|&index| state.queue.move_down(index)) {
            state.selected = index + 1;
        }
        if let Some(folder) = scan_folder {
            let capacity = player::MAX_QUEUE - state.queue.items().len();
            let token = self.background.child_token();
            let worker_token = token.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            state.scan_cancel = Some(token);
            state.scan_cancel_requested = false;
            state.queue_io = Some(rx);
            state.status = "Scanning folder…".into();
            self.background.spawn(move |_| {
                let result = player::scan_folder(&folder, capacity, &worker_token);
                let _ = tx.send(QueueIoResult::Scanned(result));
                wake.request_repaint();
            });
        }
        if cancel_scan {
            state.scan_cancel_requested = true;
            if let Some(token) = &state.scan_cancel {
                token.cancel();
            }
            state.status = "Cancelling folder scan…".into();
        }
        if let Some(action) = queue_action {
            let queue = state.queue.clone();
            let options =
                player::PlayerOptions::new(state.ffplay_path.clone(), state.volume, state.repeat);
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            state.queue_io = Some(rx);
            state.status = "Working with saved queue…".into();
            self.background.spawn(move |_| {
                let path = player::saved_queue_path();
                let result = match action {
                    QueueIoAction::Save => QueueIoResult::Saved(
                        player::save_queue(&path, &queue).map_err(|e| e.to_string()),
                    ),
                    QueueIoAction::Load => {
                        QueueIoResult::Loaded(player::load_queue(&path).map_err(|e| e.to_string()))
                    }
                    QueueIoAction::Delete => QueueIoResult::Deleted(
                        std::fs::remove_file(&path).map_err(|e| e.to_string()),
                    ),
                    QueueIoAction::SaveOptions => QueueIoResult::OptionsSaved(
                        player::save_options(&player::player_options_path(), &options)
                            .map_err(|e| e.to_string()),
                    ),
                    QueueIoAction::LoadOptions => QueueIoResult::OptionsLoaded(
                        player::load_options(&player::player_options_path())
                            .map_err(|e| e.to_string()),
                    ),
                };
                let _ = tx.send(result);
                wake.request_repaint();
            });
        }
        if start {
            let selected = state.selected;
            self.start_player_track(&mut state, selected, ctx);
        }
        if stop || skip_to.is_some() {
            state.pending = skip_to;
            state.stopping = true;
            state.status = if stop { "Stopping…" } else { "Skipping…" }.into();
            if let Some(token) = &state.cancel {
                token.cancel();
            }
        }
        self.player = state;
    }
}
