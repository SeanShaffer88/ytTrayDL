//! Controlled regression fixture. Release packaging must select --bin yttraydl.
use std::{
    io::{self, Write},
    time::Duration,
};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hang") => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        Some("stdoutcap") => {
            for _ in 0..1000 {
                println!("bounded stdout");
            }
        }
        Some("stderrcap") => {
            for _ in 0..1000 {
                eprintln!("bounded stderr");
            }
        }
        #[cfg(unix)]
        Some("escapedpipe") => {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.arg("pipeholder").arg(&args[1]);
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = command.spawn().unwrap();
            while !std::path::Path::new(&args[1]).exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
            println!("READY");
            io::stdout().flush().unwrap();
            // Remain alive until cancellation; the escaped child has a finite
            // lifetime and an explicit completion marker for test cleanup.
            let _ = child.wait();
        }
        #[cfg(unix)]
        Some("pipeholder") => {
            std::fs::write(&args[1], b"ready").unwrap();
            std::thread::sleep(Duration::from_secs(2));
            // Close inherited pipes before acknowledging fixture cleanup.
            unsafe {
                libc::close(libc::STDOUT_FILENO);
                libc::close(libc::STDERR_FILENO);
            }
            std::fs::write(format!("{}.done", args[1]), b"done").unwrap();
        }
        Some("flood") => {
            let stderr = std::thread::spawn(|| {
                let b = vec![b'E'; 8192];
                loop {
                    if io::stderr().write_all(&b).is_err() {
                        break;
                    }
                }
            });
            let b = vec![b'O'; 8192];
            loop {
                if io::stdout().write_all(&b).is_err() {
                    break;
                }
            }
            let _ = stderr.join();
        }
        Some("tree") => {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("writer")
                .arg(&args[1])
                .spawn()
                .unwrap();
            while !std::path::Path::new(&args[1]).exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
            println!("READY");
            io::stdout().flush().unwrap();
            let _ = child.wait();
        }
        Some("writer") => {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&args[1])
                .unwrap();
            loop {
                f.write_all(b"tick\n").unwrap();
                f.flush().unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Some("lines") => {
            for _ in 0..1000 {
                println!("bounded line");
                eprintln!("bounded error");
            }
        }
        Some("orphan") => {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("writer")
                .arg(&args[1])
                .spawn()
                .unwrap();
            while !std::path::Path::new(&args[1]).exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
            // Deliberately transfer cleanup to the supervisor to exercise an
            // exited root whose descendant still owns both output pipes.
            std::mem::forget(child);
            println!("root exiting");
        }
        Some("argv") => println!("{}", serde_json::to_string(&args[1..]).unwrap()),
        Some("late") => {
            println!("final output");
            eprintln!("diagnostic");
        }
        Some("exit") => std::process::exit(7),
        _ => std::process::exit(2),
    }
}
