use std::{
    io::ErrorKind,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::{Result, bail};
use nyansync::{ExtCommand, Request, Response, ResponseHeader, hex};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

pub struct Connect;

impl Connect {
    pub async fn connect(cursor: Arc<AtomicU32>, server_address: Arc<str>) {
        loop {
            let stream = match TcpStream::connect(server_address.as_ref()).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("tcp connect error: {e}");
                    return;
                }
            };

            // if error occurred, retry in this loop.
            //
            // do not spawn this task, it's only for
            // un-nest codes, originally this is a
            // loop nested in loop
            if let Err(e) = Self::handle_stream(stream, cursor.clone()).await {
                eprintln!("{e}");
            } else {
                return;
            };
        }
    }

    async fn handle_stream(mut stream: TcpStream, cursor: Arc<AtomicU32>) -> Result<()> {
        let mut write_buf = [0u8; 4];

        let mut header_buf = [0u8; ResponseHeader::TOTAL_LEN];

        loop {
            let cursor = cursor.fetch_add(1, Ordering::Relaxed);

            let req = Request::new(cursor);
            req.encode(&mut write_buf);

            if let Err(e) = stream.write_all(write_buf.as_slice()).await {
                bail!("stream write all error: {e}");
            }

            let mut first = [0u8; 1];
            if let Err(e) = stream.read_exact(&mut first).await {
                bail!("read_exact first error: {e}");
            }

            let resp = if first[0] > 127 {
                // infailable
                Response::ExtCommand(match ExtCommand::try_from(first[0]) {
                    Ok(ext_command) => ext_command,
                    Err(e) => {
                        bail!("ext_command try_from error: {e}");
                    }
                })
            } else {
                header_buf[0] = first[0];
                if let Err(e) = stream.read_exact(&mut header_buf[1..]).await {
                    bail!("read_exact header error: {e}");
                }

                match Response::decode(&header_buf) {
                    Ok(Some((resp, _))) => resp,
                    Ok(None) => unreachable!(),
                    Err(e) => {
                        bail!("response decode error: {e}");
                    }
                }
            };

            let resp_header = match resp {
                Response::Ok(response_header) => response_header,
                Response::ExtCommand(ext_command) => match ext_command {
                    ExtCommand::FileNameInvalid => continue,
                    ExtCommand::EndOfTransaction => break Ok(()),
                },
            };

            let file_name = format!("{resp_header}");
            let mut file_name_step_2_iter = file_name.as_bytes().array_windows();

            let Some(dir_first) = file_name_step_2_iter.next() else {
                bail!("unexpected file_name size");
            };
            let Some(dir_second) = file_name_step_2_iter.next() else {
                bail!("unexpected file_name size");
            };

            let dir_first: &[u8; 2] = dir_first;
            let dir_first = str::from_utf8(dir_first.as_slice()).unwrap();
            let dir_second: &[u8; 2] = dir_second;
            let dir_second = str::from_utf8(dir_second.as_slice()).unwrap();

            let dir = PathBuf::new().join(dir_first).join(dir_second);

            if let Err(e) = fs::create_dir_all(&dir).await {
                bail!("create dir all error: {e}");
            };

            let path = dir.join(&file_name);

            let mut file = match File::create_new(path).await {
                Ok(f) => f,
                Err(e) => {
                    if matches!(e.kind(), ErrorKind::AlreadyExists) {
                        bail!("file exist");
                    } else {
                        bail!("create new file error: {e}");
                    }
                }
            };

            let mut reader = (&mut stream).take(resp_header.payload_len() as u64);

            if tokio::io::copy(&mut reader, &mut file).await.is_err() {
                bail!("failed to copy reader to file");
            };

            let hash = match hex::sha1sum(&mut file).await {
                Ok(hash) => hash,
                Err(e) => {
                    bail!("sha1sum error: {e}");
                }
            };

            if resp_header.file_hash() != hash {
                bail!("file hash mismatch with file_name's hash");
            }

            let fs_size = match file.metadata().await {
                Ok(f) => f.len(),
                Err(e) => {
                    bail!("fs_size: {e}");
                }
            };

            if resp_header.payload_len() as u64 != fs_size {
                bail!("payload_len mismatch with fs_size");
            }

            eprintln!("received a new file: {file_name}");
        }
    }
}
