mod accept;
mod args;
mod path_table;

use std::{collections::BTreeSet, num::NonZero, path::Path, process::ExitCode, sync::Arc, thread};

use anyhow::Result;
use tokio::{net::TcpListener, runtime::Runtime, signal};
use walkdir::WalkDir;

use crate::{
    accept::Accept,
    args::{Args, Parse as _},
    path_table::PathTable,
};

fn main() -> Result<ExitCode> {
    let Args { root, listen } = match Args::parse() {
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

    // btree build stable file indexs
    let mut set: BTreeSet<Box<Path>> = BTreeSet::new();

    let walkdir = WalkDir::new(root.as_ref())
        .follow_root_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file());
    for entry in walkdir {
        let path = entry.path();
        set.insert(Box::from(path));
    }

    let (rt, async_main) = AsyncMain::new(
        // freeze btree into boxed slice
        PathTable::new(set)?,
        listen,
    )?;
    rt.block_on(async_main.enter())
}

struct AsyncMain {
    files: PathTable,
    listen: std::net::TcpListener,
}

impl AsyncMain {
    fn new(files: PathTable, listen: std::net::TcpListener) -> Result<(Runtime, Self)> {
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

        Ok((rt, Self { files, listen }))
    }

    async fn enter(self) -> Result<ExitCode> {
        self.listen.set_nonblocking(true)?;
        let ln = TcpListener::from_std(self.listen)?;
        let files = Arc::new(self.files);

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
