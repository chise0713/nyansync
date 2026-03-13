mod args;
mod connect;

use std::{
    num::NonZero,
    process::ExitCode,
    sync::{Arc, atomic::AtomicU32},
    thread,
};

use anyhow::Result;
use tokio::{runtime::Runtime, signal, task::JoinSet};

use crate::{
    args::{Args, Parse as _},
    connect::Connect,
};

fn main() -> Result<ExitCode> {
    let Args {
        cursor,
        server_address,
    } = match Args::parse() {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    let Some(cursor) = cursor else {
        return args::invalid_argument();
    };

    let Some(server_address) = server_address else {
        return args::invalid_argument();
    };

    let Ok(cursor) = cursor.parse() else {
        return args::invalid_argument();
    };

    let (rt, async_main) = AsyncMain::new(cursor, server_address)?;

    rt.block_on(async_main.enter())
}

struct AsyncMain {
    cursor: u32,
    server_address: Box<str>,
    worker_threads: usize,
}

impl AsyncMain {
    fn new(cursor: u32, server_address: Box<str>) -> Result<(Runtime, Self)> {
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

        Ok((
            rt,
            Self {
                cursor,
                server_address,
                worker_threads,
            },
        ))
    }

    async fn enter(self) -> Result<ExitCode> {
        let cursor = Arc::new(AtomicU32::new(self.cursor));
        let server_address: Arc<str> = self.server_address.into();

        let mut join_set = JoinSet::new();
        (0..self.worker_threads).for_each(|_| {
            _ = join_set.spawn(Connect::connect(cursor.clone(), server_address.clone()))
        });

        let mut exit_code = ExitCode::FAILURE;

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
            // do not exit when no worker thread
            Some(_) = join_set.join_next() => {},
            _ = Connect::connect(cursor, server_address) => {}
        }

        Ok(exit_code)
    }
}
