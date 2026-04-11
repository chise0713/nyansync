mod accept;
mod args;
mod path_table;

use std::{cmp::Reverse, num::NonZero, path::PathBuf, process::ExitCode, sync::Arc, thread};

use anyhow::Result;
use tokio::{net::TcpListener, runtime::Runtime, signal};
use walkdir::{DirEntry, WalkDir};

use crate::{
    accept::Accept,
    args::{Args, Parse as _},
    path_table::PathTable,
};

fn main() -> Result<ExitCode> {
    let Args {
        root,
        listen,
        timestamp,
    } = match Args::parse() {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    let Some(root) = root else {
        return args::invalid_argument();
    };

    let Some(listen) = listen else {
        return args::invalid_argument();
    };

    let listen = std::net::TcpListener::bind(listen.as_ref())?;

    let dir_entry = WalkDir::new(root.as_ref())
        .follow_root_links(false)
        .into_iter()
        .filter_map(Result::ok);
    let paths = if timestamp {
        let mut paths: Box<[_]> = dir_entry
            .filter_map(|e| {
                if !e.file_type().is_file() {
                    return None;
                }
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((mtime, e.into_path().into_boxed_path()))
            })
            .collect();
        paths.sort_unstable_by_key(|(t, _)| Reverse(*t));
        paths.into_iter().map(|(_, p)| p).collect()
    } else {
        let mut paths: Box<[_]> = dir_entry
            .filter(|e| e.file_type().is_file())
            .map(DirEntry::into_path)
            .map(PathBuf::into_boxed_path)
            .collect();
        paths.sort_unstable();
        paths
    };

    let (rt, async_main) = AsyncMain::new(PathTable::new(paths)?, listen)?;
    rt.block_on(async_main.enter())
}

struct AsyncMain {
    path_table: PathTable,
    listen: std::net::TcpListener,
}

impl AsyncMain {
    fn new(path_table: PathTable, listen: std::net::TcpListener) -> Result<(Runtime, Self)> {
        const MAIN_THREAD: usize = 1;
        // zero worker when only main thread available
        let total_threads = thread::available_parallelism()
            .map(NonZero::get)
            .unwrap_or(1);
        let worker_threads = total_threads.saturating_sub(MAIN_THREAD);

        let rt = if worker_threads == 0 {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
        } else {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(worker_threads)
                .enable_all()
                .build()?
        };

        Ok((rt, Self { path_table, listen }))
    }

    async fn enter(self) -> Result<ExitCode> {
        self.listen.set_nonblocking(true)?;
        let ln = TcpListener::from_std(self.listen)?;
        let files = Arc::new(self.path_table);

        eprintln!("service started");

        let mut exit_code = ExitCode::FAILURE;

        let task = async {
            loop {
                let (stream, addr) = match ln.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("accept error: {e}");
                        return;
                    }
                };
                eprintln!("new client: {addr}");
                tokio::spawn(Accept::accept(stream, addr, files.clone()));
            }
        };

        tokio::select! {
            r = signal::ctrl_c() => {
                match r {
                    Ok(()) => {
                        eprintln!("shutting down");
                        exit_code = ExitCode::SUCCESS;
                    },
                    Err(e) => eprintln!("{e}"),
                }
            },
            _ = task => {}
        }

        Ok(exit_code)
    }
}
