use crate::fixtures::Server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use yttraydl::download::{DownloadEvent, DownloadManager, Job, JobState};
use yttraydl::history::{History, HistoryEntry, HistoryStatus};
use yttraydl::process::{CancellationToken, OwnedProcess, ProcessLimits, ProcessSpec};
use yttraydl::settings::{Preset, Settings};

type Result<T> = std::result::Result<T, String>;
const SCENARIOS: &[&str] = &[
    "video",
    "merge",
    "audio",
    "subtitles",
    "playlist",
    "retry",
    "pause",
    "cancel",
    "shutdown",
];

struct Options {
    ytdlp: PathBuf,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    root: PathBuf,
    scenario: String,
    timeout: Duration,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let mut values = std::collections::BTreeMap::new();
        while let Some(key) = args.next() {
            if ![
                "--yt-dlp",
                "--ffmpeg",
                "--ffprobe",
                "--root",
                "--scenario",
                "--timeout-seconds",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unknown option {key}"));
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {key}"))?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate {key}"));
            }
        }
        let get = |key: &str| {
            values
                .get(key)
                .cloned()
                .ok_or_else(|| format!("required: {key}"))
        };
        let executable = |key: &str| -> Result<PathBuf> {
            let path = PathBuf::from(get(key)?);
            if !path.is_absolute() || !path.is_file() {
                return Err(format!("{key} must name an existing absolute executable"));
            }
            Ok(path)
        };
        let root = PathBuf::from(get("--root")?);
        if !root.is_absolute() || root.exists() {
            return Err("--root must be a new absolute directory".into());
        }
        let scenario = get("--scenario")?;
        if scenario != "all" && !SCENARIOS.contains(&scenario.as_str()) {
            return Err(format!("unknown scenario {scenario}"));
        }
        let seconds = values
            .get("--timeout-seconds")
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or(120);
        if !(10..=120).contains(&seconds) {
            return Err("timeout must be 10..120 seconds per scenario".into());
        }
        Ok(Self {
            ytdlp: executable("--yt-dlp")?,
            ffmpeg: executable("--ffmpeg")?,
            ffprobe: executable("--ffprobe")?,
            root,
            scenario,
            timeout: Duration::from_secs(seconds),
        })
    }
}

struct Harness {
    options: Options,
    evidence: Value,
}

impl Harness {
    fn save(&self) -> Result<()> {
        std::fs::write(
            self.options.root.join("evidence.json"),
            serde_json::to_vec_pretty(&self.evidence).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }

    fn command(&mut self, program: &Path, args: &[String], timeout: Duration) -> Result<Vec<u8>> {
        let process = OwnedProcess::spawn(&ProcessSpec::new(program).args(args))
            .map_err(|e| e.to_string())?;
        let pid = process.id();
        let result = process.capture(
            ProcessLimits {
                timeout,
                ..Default::default()
            },
            &CancellationToken::default(),
        );
        let record = match &result {
            Ok(output) => {
                json!({"program":program,"args":args,"pid":pid,"exitCode":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
            }
            Err(error) => {
                json!({"program":program,"args":args,"pid":pid,"exitCode":null,"error":error.to_string()})
            }
        };
        self.evidence["commands"]
            .as_array_mut()
            .unwrap()
            .push(record);
        self.save()?;
        let output = result.map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{} exit {:?}: {}",
                program.display(),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(output.stdout)
    }

    fn setup(&mut self) -> Result<PathBuf> {
        let root = self.options.root.join("fixtures");
        std::fs::create_dir(&root).map_err(|e| e.to_string())?;
        for (tool, flag) in [
            (self.options.ytdlp.clone(), "--version"),
            (self.options.ffmpeg.clone(), "-version"),
            (self.options.ffprobe.clone(), "-version"),
        ] {
            self.command(&tool, &[flag.into()], Duration::from_secs(20))?;
        }
        let ffmpeg = self.options.ffmpeg.clone();
        let video = root.join("video.mp4");
        let audio = root.join("audio.m4a");
        let combined = root.join("combined.mp4");
        for (args, path) in [
            (
                vec![
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=160x90:rate=15",
                    "-t",
                    "8",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-pix_fmt",
                    "yuv420p",
                ],
                &video,
            ),
            (
                vec![
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100",
                    "-t",
                    "8",
                    "-vn",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "64k",
                ],
                &audio,
            ),
        ] {
            let mut arguments = strings(&["-hide_banner", "-nostdin", "-loglevel", "error"]);
            arguments.extend(args.into_iter().map(str::to_owned));
            arguments.push(path.to_string_lossy().into_owned());
            self.command(&ffmpeg, &arguments, Duration::from_secs(30))?;
        }
        self.command(
            &ffmpeg,
            &strings(&[
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-i",
                &video.to_string_lossy(),
                "-i",
                &audio.to_string_lossy(),
                "-c",
                "copy",
                &combined.to_string_lossy(),
            ]),
            Duration::from_secs(30),
        )?;
        std::fs::write(
            root.join("captions.vtt"),
            "WEBVTT\n\n00:00:00.000 --> 00:00:04.000\nOriginal café field caption.\n",
        )
        .map_err(|e| e.to_string())?;
        self.command(
            &ffmpeg,
            &strings(&[
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-i",
                &combined.to_string_lossy(),
                "-vf",
                "hue=h=90",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "copy",
                &root.join("combined-two.mp4").to_string_lossy(),
            ]),
            Duration::from_secs(30),
        )?;
        Ok(root)
    }

    fn settings(&self, root: &Path, info: Option<&Path>) -> Settings {
        let mut extra =
            "--ignore-config --no-cache-dir --socket-timeout 5 --retries 0 --fragment-retries 0"
                .to_owned();
        if let Some(info) = info {
            extra.push_str(&format!(" --load-info-json \"{}\"", info.display()));
        }
        Settings {
            download_folder: root.into(),
            ffmpeg_path: self.options.ffmpeg.clone(),
            ytdlp_path: self.options.ytdlp.clone(),
            embed_metadata: false,
            embed_thumbnail: false,
            max_concurrent_downloads: 1,
            subtitle_languages: "en".into(),
            extra_args: extra,
            ..Settings::default()
        }
    }

    fn probe(&mut self, path: &Path, video: bool, audio: bool) -> Result<Value> {
        require(
            path.is_file() && std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 0,
            "final media exists and is nonempty",
        )?;
        let probe = self.options.ffprobe.clone();
        let bytes = self.command(
            &probe,
            &strings(&[
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
                &path.to_string_lossy(),
            ]),
            Duration::from_secs(15),
        )?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let streams = value["streams"]
            .as_array()
            .ok_or("ffprobe omitted streams")?;
        require(
            streams.iter().any(|s| s["codec_type"] == "video") == video,
            "expected video stream presence",
        )?;
        require(
            streams.iter().any(|s| s["codec_type"] == "audio") == audio,
            "expected audio stream presence",
        )?;
        let duration = value["format"]["duration"]
            .as_str()
            .ok_or("missing duration")?
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        require(
            (7.5..=9.0).contains(&duration),
            "media duration approximately 8 seconds",
        )?;
        Ok(value)
    }

    fn run_job(
        &mut self,
        settings: &Settings,
        url: &str,
        preset: Preset,
        playlist: Option<bool>,
        intervention: Option<&str>,
    ) -> Result<Job> {
        let mut manager = DownloadManager::new();
        let id = manager.enqueue_full(url.into(), String::new(), preset, false, 0, playlist);
        let args =
            yttraydl::ytdlp::build_download_args_full(settings, preset, url, false, playlist, None);
        self.evidence["downloads"].as_array_mut().unwrap().push(
            json!({"id":id,"url":url,"args":args,"intervention":intervention,"attemptResults":[]}),
        );
        let index = self.evidence["downloads"].as_array().unwrap().len() - 1;
        let start = Instant::now();
        let mut intervened = false;
        let mut observed = Vec::new();
        let mut history = History::default();
        manager.pump(settings, Some(&self.options.ytdlp));
        loop {
            for event in manager.poll_events() {
                if let DownloadEvent::Finished(job) = event {
                    self.evidence["downloads"][index]["lastJob"] = json!({"state":format!("{:?}",job.state),"attempts":job.attempts,"file":job.final_file,"tail":job.tail_lines,"error":job.error});
                    let outcome = self.evidence["downloads"][index]["lastJob"].clone();
                    self.evidence["downloads"][index]["attemptResults"]
                        .as_array_mut()
                        .unwrap()
                        .push(outcome);
                    self.save()?;
                    if job.state == JobState::Paused {
                        require(
                            intervention == Some("pause") && intervened,
                            "pause after observed ffmpeg",
                        )?;
                        require(
                            history.entries.is_empty(),
                            "pause writes no terminal history",
                        )?;
                        self.assert_gone(&observed)?;
                        require(manager.resume(id), "paused job resumes")?;
                        manager.pump(settings, Some(&self.options.ytdlp));
                        continue;
                    }
                    if let Some(next) = job.automatic_retry_attempt(settings.auto_retry_attempts) {
                        manager.enqueue_job(
                            job.url.clone(),
                            job.title.clone(),
                            job.preset,
                            job.use_archive,
                            next,
                            job.playlist_all,
                            job.output_dir.clone(),
                        );
                        manager.pump(settings, Some(&self.options.ytdlp));
                        continue;
                    }
                    history.push(HistoryEntry {
                        finished_at_unix: yttraydl::history::now_unix(),
                        url: job.url.clone(),
                        title: job.title.clone(),
                        status: match job.state {
                            JobState::Done => HistoryStatus::Completed,
                            JobState::Cancelled => HistoryStatus::Cancelled,
                            _ => HistoryStatus::Failed,
                        },
                        file: job.final_file.clone(),
                        error: job.error.clone(),
                        preset_label: job.preset.label().into(),
                    });
                    require(
                        history.entries.len() == 1,
                        "one terminal history result after automatic retries",
                    )?;
                    history.save().map_err(|e| e.to_string())?;
                    let persisted = History::load();
                    require(
                        persisted.entries.len() == 1
                            && persisted.entries[0].file == job.final_file
                            && persisted.entries[0].status == history.entries[0].status,
                        "private persisted terminal history retains final path and status",
                    )?;
                    self.evidence["downloads"][index]["history"] =
                        serde_json::to_value(&history).map_err(|e| e.to_string())?;
                    if intervention.is_some() {
                        require(
                            intervened,
                            "actual ffmpeg descendant observed before intervention",
                        )?;
                        self.assert_gone(&observed)?;
                    }
                    self.save()?;
                    return Ok(job);
                }
            }
            if intervention.is_some() && !intervened {
                let pids = self.ffmpeg_descendants()?;
                if !pids.is_empty() {
                    observed = pids;
                    self.evidence["downloads"][index]["observedFfmpegPids"] = json!(observed);
                    intervened = true;
                    match intervention {
                        Some("pause") => manager.pause(id),
                        Some("cancel") => manager.cancel(id),
                        Some("shutdown") => manager.shutdown(),
                        Some("observe") => {}
                        _ => unreachable!(),
                    }
                }
            }
            if start.elapsed() > self.options.timeout {
                return Err(
                    "scenario deadline exceeded; manager drop terminates owned tree".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn ffmpeg_descendants(&mut self) -> Result<Vec<u32>> {
        if !cfg!(windows) {
            return Err("Unavailable: lifecycle observation requires Windows".into());
        }
        // Filter by owned parent identity at every level; never enumerate user command lines.
        let script = format!("$ErrorActionPreference='Stop'; $parents=@({}); $found=@(); for($depth=0;$depth -lt 5;$depth++){{$next=@();foreach($parent in $parents){{$children=@(Get-CimInstance Win32_Process -Filter \"ParentProcessId=$parent\");foreach($child in $children){{if($child.Name -eq 'ffmpeg.exe'){{$found+=[int]$child.ProcessId}}; $next+=[int]$child.ProcessId}}}};$parents=$next;if($parents.Count -eq 0){{break}}}}; ConvertTo-Json -Compress -InputObject @($found)", std::process::id());
        let output = self.powershell(&script)?;
        serde_json::from_slice(&output).map_err(|e| e.to_string())
    }

    fn powershell(&mut self, script: &str) -> Result<Vec<u8>> {
        self.command(
            Path::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"),
            &strings(&["-NoProfile", "-NonInteractive", "-Command", script]),
            Duration::from_secs(10),
        )
    }

    fn assert_gone(&mut self, pids: &[u32]) -> Result<()> {
        let start = Instant::now();
        loop {
            let ids = pids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let output = self.powershell(&format!(
                "@(Get-Process -Id {ids} -ErrorAction SilentlyContinue).Count"
            ))?;
            if String::from_utf8_lossy(&output).trim() == "0" {
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err(format!(
                    "ffmpeg descendants survived cleanup deadline: {pids:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn scenario(&mut self, name: &str, server: &Server) -> Result<Value> {
        let root = self.options.root.join(name);
        std::fs::create_dir(&root).map_err(|e| e.to_string())?;
        if name == "retry" {
            return self.retry(server, &root);
        }
        if name == "playlist" {
            return self.playlist(server, &root);
        }
        let separate = matches!(name, "merge" | "pause" | "cancel" | "shutdown");
        let info = root.join("input.info.json");
        write_json(&info, &server.info(name, separate))?;
        let mut settings = self.settings(&root, Some(&info));
        let url = format!("{}/combined.mp4", server.base);
        if name == "merge" {
            settings
                .extra_args
                .push_str(" --postprocessor-args \"Merger+ffmpeg_i1:-re\"");
        }
        if matches!(name, "pause" | "cancel" | "shutdown") {
            settings
                .extra_args
                .push_str(" --postprocessor-args \"Merger+ffmpeg_i1:-re\"");
            let job = self.run_job(&settings, &url, Preset::Best, None, Some(name))?;
            if name == "pause" {
                require(job.state == JobState::Done, "resumed job completes")?;
                return self.probe(
                    job.final_file
                        .as_deref()
                        .ok_or("missing resumed final path")?,
                    true,
                    true,
                );
            }
            require(
                job.state == JobState::Cancelled,
                "interrupted job cancelled, never completed",
            )?;
            return Ok(json!({"state":format!("{:?}",job.state)}));
        }
        let preset = match name {
            "audio" => Preset::AudioMp3,
            "subtitles" => Preset::SubtitlesOnly,
            _ => Preset::Best,
        };
        let job = self.run_job(
            &settings,
            &url,
            preset,
            None,
            (name == "merge").then_some("observe"),
        )?;
        require(
            job.state == JobState::Done,
            &format!("real download succeeds: {:?}", job.error),
        )?;
        let path = job.final_file.as_deref().ok_or("missing final path")?;
        let probe = if name == "subtitles" {
            require(
                path.extension().is_some_and(|e| e == "srt"),
                "subtitle-only final path is an actual SRT",
            )?;
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            require(
                text.contains("Original café field caption."),
                "converted English subtitle content",
            )?;
            json!({"file":path,"caption":text})
        } else {
            self.probe(path, name != "audio", true)?
        };
        if name == "audio" {
            require(
                path.extension().is_some_and(|e| e == "mp3"),
                "MP3 output extension",
            )?;
            require(probe["streams"][0]["codec_name"] == "mp3", "MP3 codec")?;
        }
        self.literal_hook(&root, &job)?;
        Ok(probe)
    }

    fn literal_hook(&mut self, root: &Path, job: &Job) -> Result<()> {
        let path = job.final_file.as_deref().ok_or("missing hook final path")?;
        require(path.is_file(), "hook final path exists")?;
        let hook_file = root.join("hook.json");
        let template = format!(
            "\"{}\" --record-hook \"{}\" \"{{file}}\"",
            std::env::current_exe()
                .map_err(|e| e.to_string())?
                .display(),
            hook_file.display()
        );
        let hook = yttraydl::hooks::HookSpec::parse(&template)?.expand(
            &path.to_string_lossy(),
            &root.to_string_lossy(),
            &job.url,
            &job.title,
        );
        self.command(
            Path::new(&hook.executable),
            &hook.args,
            Duration::from_secs(10),
        )?;
        let recorded: Value =
            serde_json::from_slice(&std::fs::read(hook_file).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        require(
            recorded == json!([path]),
            "literal hook receives final Unicode/spaced path",
        )?;
        Ok(())
    }

    fn retry(&mut self, server: &Server, root: &Path) -> Result<Value> {
        for (route, transport, auto, state, attempts, requests) in [
            ("transport.mp4", 1, 0, JobState::Done, 0, 2),
            ("app-retry.mp4", 0, 1, JobState::Done, 1, 2),
            ("exhaust.mp4", 0, 2, JobState::Failed, 2, 3),
        ] {
            let info = root.join(format!("{route}.info.json"));
            let mut value = server.info(route, false);
            value["formats"][0]["url"] = json!(format!("{}/{route}", server.base));
            write_json(&info, &value)?;
            let mut settings = self.settings(root, Some(&info));
            settings
                .extra_args
                .push_str(&format!(" --retries {transport} --retry-sleep 0"));
            settings.auto_retry_attempts = auto;
            let job = self.run_job(
                &settings,
                &format!("{}/{route}", server.base),
                Preset::Best,
                None,
                None,
            )?;
            require(
                job.state == state && job.attempts == attempts,
                "independent transport/application retry budgets",
            )?;
            require(
                server
                    .counts
                    .lock()
                    .unwrap()
                    .get(&format!("/{route}"))
                    .copied()
                    == Some(requests),
                "exact controlled request count",
            )?;
            if job.state == JobState::Done {
                self.probe(
                    job.final_file
                        .as_deref()
                        .ok_or("retry final path missing")?,
                    true,
                    true,
                )?;
            }
        }
        Ok(json!({"requests":*server.counts.lock().unwrap()}))
    }

    fn playlist(&mut self, server: &Server, root: &Path) -> Result<Value> {
        for (all, expected) in [(true, 2), (false, 1)] {
            let output = root.join(if all { "all" } else { "single" });
            std::fs::create_dir(&output).map_err(|e| e.to_string())?;
            let settings = self.settings(&output, None);
            let job = self.run_job(
                &settings,
                &format!("{}/playlist.html", server.base),
                Preset::Best,
                Some(all),
                None,
            )?;
            require(job.state == JobState::Done, "playlist completes")?;
            let files = std::fs::read_dir(&output)
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "mp4"))
                .collect::<Vec<_>>();
            require(
                files.len() == expected,
                &format!(
                    "playlist {all}: expected {expected} outputs, got {}",
                    files.len()
                ),
            )?;
            require(
                job.final_file
                    .as_ref()
                    .is_some_and(|path| files.contains(path)),
                "playlist final path names an actual completed item",
            )?;
            self.literal_hook(&output, &job)?;
            if all {
                require(
                    std::fs::read(&files[0]).map_err(|e| e.to_string())?
                        != std::fs::read(&files[1]).map_err(|e| e.to_string())?,
                    "playlist items have distinct original media content",
                )?;
            }
            for path in files {
                self.probe(&path, true, true)?;
            }
        }
        Ok(json!({"allItems":2,"singleItem":1}))
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
fn require(condition: bool, assertion: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(format!("assertion failed: {assertion}"))
    }
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn main() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--record-hook") {
        if args.len() != 4 {
            return 2;
        }
        return if write_json(Path::new(&args[2]), &json!(&args[3..])).is_ok() {
            0
        } else {
            1
        };
    }
    let options = match Options::parse() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("Unavailable: {e}");
            return 2;
        }
    };
    if let Err(e) = std::fs::create_dir(&options.root) {
        eprintln!("Unavailable: {e}");
        return 2;
    }
    // Set before any worker threads or Settings/config OnceLock access. Never
    // consult or append to the interactive user's archive or diagnostic log.
    let executable = std::env::current_exe().expect("current executable");
    if executable.parent().unwrap().join("portable.txt").exists() {
        eprintln!(
            "Unavailable: harness executable must not share a portable application directory"
        );
        return 2;
    }
    std::env::set_var("APPDATA", options.root.join("profile"));
    let mut harness = Harness {
        options,
        evidence: json!({"schemaVersion":1,"coverage":"real DownloadManager + loopback download/postprocessing; external discovery unverified; history/hook library consumer; GUI integration separately required","pid":std::process::id(),"os":std::env::consts::OS,"commands":[],"downloads":[],"scenarios":[],"status":"running"}),
    };
    let result = (|| -> Result<()> {
        let fixtures = harness.setup()?;
        let server = Server::start(&fixtures)?;
        write_json(
            &harness.options.root.join("fixture-server.json"),
            &json!({"base":server.base,"root":fixtures,"ownerPid":std::process::id()}),
        )?;
        let selected = if harness.options.scenario == "all" {
            SCENARIOS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        } else {
            vec![harness.options.scenario.clone()]
        };
        let mut failed = false;
        for scenario in selected {
            let record = match harness.scenario(&scenario, &server) {
                Ok(evidence) => {
                    json!({"scenario":scenario,"status":"passed","assertions":evidence})
                }
                Err(error) => {
                    failed = true;
                    json!({"scenario":scenario,"status":"failed","error":error})
                }
            };
            println!("{record}");
            harness.evidence["scenarios"]
                .as_array_mut()
                .unwrap()
                .push(record);
            harness.save()?;
        }
        harness.evidence["requests"] = json!(*server.counts.lock().unwrap());
        require(!failed, "all selected scenarios passed")
    })();
    harness.evidence["status"] = json!(if result.is_ok() { "passed" } else { "failed" });
    if let Err(e) = &result {
        harness.evidence["error"] = json!(e);
        eprintln!("{e}");
    }
    println!(
        "evidence: {}",
        harness.options.root.join("evidence.json").display()
    );
    if let Err(e) = harness.save() {
        eprintln!("evidence write failed: {e}");
        return 1;
    }
    if result.is_ok() {
        0
    } else {
        1
    }
}
