//! Original synthetic media and a bounded, owned loopback fixture server.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone)]
pub struct Route {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub failures: usize,
}

pub struct Server {
    pub base: String,
    pub counts: Arc<Mutex<BTreeMap<String, usize>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(root: &Path) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let base = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let mut routes = BTreeMap::new();
        for (name, mime) in [
            ("video.mp4", "video/mp4"),
            ("audio.m4a", "audio/mp4"),
            ("combined.mp4", "video/mp4"),
            ("combined-two.mp4", "video/mp4"),
            ("captions.vtt", "text/vtt"),
        ] {
            routes.insert(
                format!("/{name}"),
                Route {
                    bytes: std::fs::read(root.join(name)).map_err(|e| e.to_string())?,
                    content_type: mime,
                    failures: 0,
                },
            );
        }
        for (name, failures) in [
            ("transport.mp4", 1),
            ("app-retry.mp4", 1),
            ("exhaust.mp4", 100),
        ] {
            routes.insert(
                format!("/{name}"),
                Route {
                    bytes: routes["/combined.mp4"].bytes.clone(),
                    content_type: "video/mp4",
                    failures,
                },
            );
        }
        let playlist = format!("<!doctype html><title>Original field playlist</title><video controls src=\"{base}/combined.mp4?one\"></video><video controls src=\"{base}/combined-two.mp4?two\"></video>");
        routes.insert(
            "/playlist.html".into(),
            Route {
                bytes: playlist.into_bytes(),
                content_type: "text/html",
                failures: 0,
            },
        );
        let stop = Arc::new(AtomicBool::new(false));
        let counts = Arc::new(Mutex::new(BTreeMap::new()));
        let worker_stop = stop.clone();
        let worker_counts = counts.clone();
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Err(error) = serve(&mut stream, &routes, &worker_counts) {
                            eprintln!("fixture serve: {error}");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    // Windows accept can report a reset/aborted pending peer.
                    // A single disconnected HTTP client must not shut down the
                    // fixture listener for every later scenario.
                    Err(error) => {
                        eprintln!("fixture accept: {error}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });
        Ok(Self {
            base,
            counts,
            stop,
            worker: Some(worker),
        })
    }

    pub fn info(&self, id: &str, separate: bool) -> Value {
        let mut info = json!({
            "id": id, "title": format!("Original café field {id}"),
            "extractor": "field_fixture", "extractor_key": "Generic",
            "webpage_url": format!("{}/combined.mp4", self.base),
            "duration": 8,
            "subtitles": {"en": [{"url": format!("{}/captions.vtt", self.base), "ext": "vtt"}]}
        });
        info["formats"] = if separate {
            json!([
                {"format_id":"v", "url":format!("{}/video.mp4",self.base), "ext":"mp4", "vcodec":"h264", "acodec":"none", "height":90, "width":160},
                {"format_id":"a", "url":format!("{}/audio.m4a",self.base), "ext":"m4a", "vcodec":"none", "acodec":"aac", "abr":64}
            ])
        } else {
            json!([{"format_id":"combined", "url":format!("{}/combined.mp4",self.base), "ext":"mp4", "vcodec":"h264", "acodec":"aac", "height":90, "width":160}])
        };
        info
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(
    stream: &mut TcpStream,
    routes: &BTreeMap<String, Route>,
    counts: &Mutex<BTreeMap<String, usize>>,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let mut buf = [0; 2048];
    while bytes.len() < 16 * 1024 {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&bytes);
    let mut words = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = words.next().unwrap_or_default();
    let path = words
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    let count = {
        let mut counts = counts.lock().unwrap();
        let count = counts.entry(path.into()).or_insert(0);
        *count += 1;
        *count
    };
    let Some(route) = routes.get(path) else {
        return stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    if count <= route.failures {
        return stream.write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    let start = request
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range: bytes=")
                .or_else(|| line.strip_prefix("range: bytes="))
        })
        .and_then(|r| r.split('-').next()?.parse::<usize>().ok())
        .unwrap_or(0);
    if start >= route.bytes.len() {
        return stream.write_all(
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    let status = if start == 0 {
        "200 OK"
    } else {
        "206 Partial Content"
    };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n", route.content_type, route.bytes.len() - start)?;
    if start > 0 {
        write!(
            stream,
            "Content-Range: bytes {start}-{}/{}\r\n",
            route.bytes.len() - 1,
            route.bytes.len()
        )?;
    }
    stream.write_all(b"\r\n")?;
    if method != "HEAD" {
        stream.write_all(&route.bytes[start..])?;
    }
    Ok(())
}
