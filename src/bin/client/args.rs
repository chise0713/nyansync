use std::process::ExitCode;

use anyhow::Result;

#[derive(supershorty::Args, Debug)]
#[args(name = "nyansync-server")]
pub struct Args {
    #[arg(flag = 'c', help = "cursor start")]
    pub cursor: Option<u32>,
    #[arg(flag = 't', help = "multi thread download task count")]
    pub task_count: Option<u32>,
    #[arg(flag = 's', help = "server address")]
    pub server_address: Option<Box<str>>,
}

const EXIT_INVALID_ARG: u8 = 2;

pub fn invalid_argument() -> Result<ExitCode> {
    Args::usage();
    Ok(ExitCode::from(EXIT_INVALID_ARG))
}
