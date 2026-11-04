//! Native yt-dlp-shaped fixture, explicitly compiled by manager regressions.
use std::{io::Write, time::Duration};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "writer") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&args[1])
            .unwrap();
        loop {
            file.write_all(b"tick\n").unwrap();
            file.flush().unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let mode = args.last().unwrap();
    if mode == "probe-hang" {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    } else if mode == "probe-healthy" {
        println!(
            "{{\"id\":\"healthy\",\"url\":\"https://example.test/healthy\",\"title\":\"healthy\"}}"
        );
    } else if let Some(path) = mode.strip_prefix("tree:") {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["writer", path])
            .spawn()
            .unwrap();
        while !std::path::Path::new(path).exists() {
            std::thread::sleep(Duration::from_millis(2));
        }
        println!("READY");
        std::io::stdout().flush().unwrap();
        let _ = child.wait();
    } else {
        println!("[download] Destination: video.f137.mp4");
        println!("[download] Destination: video.f140.m4a");
        println!("[Merger] Merging formats into \"video.mp4\"");
        if mode == "extract" {
            println!("[ExtractAudio] Destination: video.mp3");
        }
        // Force a split line, then a final marker without a trailing newline.
        print!("yttraydl:fi");
        std::io::stdout().flush().unwrap();
        print!(
            "nal:{}",
            if mode == "extract" {
                "video.mp3"
            } else {
                "video.mp4"
            }
        );
        std::io::stdout().flush().unwrap();
    }
}
