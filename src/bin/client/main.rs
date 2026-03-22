mod args;
mod connect;

use std::{
    num::NonZero,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};

use anyhow::Result;
use tokio::{
    runtime::Runtime,
    signal,
    task::{JoinSet, LocalSet},
};

use crate::{
    args::{Args, Parse as _},
    connect::Connect,
};

fn main() -> Result<ExitCode> {
    let Args {
        cursor,
        server_address,
        task_count,
    } = match Args::parse() {
        Ok(v) => v,
        Err(e) => return Ok(e),
    };

    let cursor = cursor.unwrap_or_default();

    let Some(server_address) = server_address else {
        return args::invalid_argument();
    };

    let task_count = task_count.unwrap_or_default();

    let (rt, async_main) = AsyncMain::new(cursor, server_address, task_count)?;

    rt.block_on(async_main.enter())
}

struct AsyncMain {
    cursor: u32,
    server_address: Box<str>,
    task_count: usize,
}

impl AsyncMain {
    fn new(cursor: u32, server_address: Box<str>, task_count: u32) -> Result<(Runtime, Self)> {
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

        let task_count = if task_count == 0 {
            worker_threads + 1
        } else {
            task_count as usize
        };

        Ok((
            rt,
            Self {
                cursor,
                server_address,
                task_count,
            },
        ))
    }

    async fn enter(self) -> Result<ExitCode> {
        let cursor = Arc::new(AtomicU32::new(self.cursor));
        let server_address: Arc<str> = self.server_address.into();

        let mut join_set = JoinSet::new();
        (0..self.task_count - 1).for_each(|_| {
            _ = join_set.spawn(Connect::connect(cursor.clone(), server_address.clone()))
        });

        let local_set = LocalSet::new();
        join_set.spawn_local_on(Connect::connect(cursor.clone(), server_address), &local_set);

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
            e = local_set.run_until(join_set.join_all()) => {
                // if all booleans are `true`
                if e.into_iter().all(|e| e) {
                    eprintln!("end of transaction");
                    exit_code = ExitCode::SUCCESS;
                }
            },
        }

        eprintln!("cursor ends at: {}", cursor.load(Ordering::Relaxed));

        Ok(exit_code)
    }
}
