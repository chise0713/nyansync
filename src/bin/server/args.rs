use std::process::ExitCode;

use anyhow::Result;

#[derive(supershorty::Args, Debug)]
#[args(name = "nyansync-server")]
pub struct Args {
    #[arg(flag = 'r', help = "hath root path")]
    pub root: Option<Box<str>>,
    #[arg(flag = 'l', help = "listen address")]
    pub listen: Option<Box<str>>,
    #[arg(flag = 't', help = "sort using timestamp")]
    pub timestamp: bool,
}

const EXIT_INVALID_ARG: u8 = 2;

pub fn invalid_argument() -> Result<ExitCode> {
    Args::usage();
    Ok(ExitCode::from(EXIT_INVALID_ARG))
}
