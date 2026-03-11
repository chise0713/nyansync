mod accept;
mod args;

use std::{collections::BTreeSet, num::NonZero, path::Path, process::ExitCode, sync::Arc, thread};

use anyhow::Result;
use tokio::{
    net::TcpListener,
    runtime::Runtime,
    sync::{Semaphore, broadcast},
    task::JoinSet,
};
use walkdir::WalkDir;

use crate::{
    accept::Accept,
    args::{Args, Parse as _},
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

    let mut set: BTreeSet<Box<Path>> = BTreeSet::new();

    let walkdir = WalkDir::new(root.as_ref())
        .follow_root_links(false)
        .into_iter()
        .filter_map(Result::ok);
    for entry in walkdir {
        let path = entry.path();
        set.insert(Box::from(path));
    }

    let (rt, async_main) = AsyncMain::new(set, listen)?;
    rt.block_on(async_main.enter())
}

struct AsyncMain {
    set: BTreeSet<Box<Path>>,
    listen: std::net::TcpListener,
    worker_threads: usize,
}

impl AsyncMain {
    fn new(set: BTreeSet<Box<Path>>, listen: std::net::TcpListener) -> Result<(Runtime, Self)> {
        const MAIN_THREAD: usize = 1;
        // zero worker when only main thread available
        let total_threads = thread::available_parallelism()
            .map(NonZero::get)
            .unwrap_or(1);
        let worker_threads = total_threads.saturating_sub(MAIN_THREAD);

        Ok((
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?,
            Self {
                set,
                listen,
                worker_threads,
            },
        ))
    }

    async fn enter(self) -> Result<ExitCode> {
        let ln = Arc::new(TcpListener::from_std(self.listen)?);
        let semaphore = Arc::new(Semaphore::new(self.worker_threads + 1));
        let set = Arc::new(self.set);
        let (tx, rx) = broadcast::channel(self.worker_threads + 1);

        let mut join_set = JoinSet::new();

        (0..self.worker_threads).for_each(|_| {
            _ = join_set.spawn(Accept::accept(
                ln.clone(),
                set.clone(),
                semaphore.clone(),
                tx.clone(),
                tx.subscribe(),
            ))
        });

        let exit_code = ExitCode::FAILURE;

        tokio::select! {
            _ = join_set.join_next() => {

            },
            _ = Accept::accept(ln, set, semaphore, tx, rx) => {

            }
        }

        Ok(exit_code)
    }
}
