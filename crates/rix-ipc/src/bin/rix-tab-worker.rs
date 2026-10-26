
#![deny(unsafe_code)]

use std::path::PathBuf;

fn usage() -> String {
    "usage: rix-tab-worker --socket <path> --tab-id <id> --shmem <path>".to_string()
}

fn parse_args() -> Result<(PathBuf, u32, PathBuf), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut socket: Option<PathBuf> = None;
    let mut tab_id: Option<u32> = None;
    let mut shmem: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage());
                }
                socket = Some(PathBuf::from(&args[i]));
            }
            "--tab-id" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage());
                }
                tab_id = Some(args[i].parse::<u32>().map_err(|e| e.to_string())?);
            }
            "--shmem" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage());
                }
                shmem = Some(PathBuf::from(&args[i]));
            }
            other => return Err(format!("unknown arg: {}\n{}", other, usage())),
        }
        i += 1;
    }

    match (socket, tab_id, shmem) {
        (Some(s), Some(t), Some(m)) => Ok((s, t, m)),
        _ => Err(usage()),
    }
}

#[tokio::main]
async fn main() {
    let (socket_path, tab_id, shmem_path) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rix-tab-worker: {}", e);
            std::process::exit(2);
        }
    };

    if let Err(e) =
        rix_ipc::process::run_tab_worker_process(&socket_path, tab_id, &shmem_path).await
    {
        eprintln!("rix-tab-worker (tab {}): {}", tab_id, e);
        std::process::exit(1);
    }
}
