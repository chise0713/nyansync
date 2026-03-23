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
    fs::{self, OpenOptions},
    io::{self, AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

pub struct Connect;

impl Connect {
    pub async fn connect(
        cursor: Arc<AtomicU32>,
        server_address: Arc<str>,
        override_files: bool,
    ) -> bool {
        loop {
            let mut stream = match TcpStream::connect(server_address.as_ref()).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("tcp connect error: {e}");
                    return false;
                }
            };
            eprintln!("connected to {server_address}");

            match Self::handle_stream(&mut stream, cursor.clone(), override_files).await {
                Ok(v) => {
                    _ = stream.shutdown().await;
                    return v;
                }
                Err(e) => eprintln!("{e}"),
            }
        }
    }

    async fn handle_stream(
        stream: &mut TcpStream,
        cursor: Arc<AtomicU32>,
        override_files: bool,
    ) -> Result<bool> {
        let mut write_buf = [0u8; 4];

        let mut header_buf = [0u8; ResponseHeader::TOTAL_LEN];

        loop {
            let cursor = cursor.fetch_add(1, Ordering::Relaxed);

            let req = Request::new(cursor);
            req.encode(&mut write_buf);

            if let Err(e) = stream.write_all(write_buf.as_slice()).await {
                bail!("stream write all error: {e}");
            }

            if let Err(e) = stream.read_exact(&mut header_buf[..1]).await {
                bail!("read_exact first error: {e}");
            }

            let resp = if header_buf[0] > 127 {
                Response::ExtCommand(match ExtCommand::try_from(header_buf[0]) {
                    Ok(ext_command) => ext_command,
                    Err(e) => {
                        bail!("ext_command try_from error: {e}");
                    }
                })
            } else {
                if let Err(e) = stream.read_exact(&mut header_buf[1..]).await {
                    bail!("read_exact header error: {e}");
                }

                match Response::decode(&header_buf) {
                    Ok(Some((resp, _))) => resp,
                    Ok(None) => unreachable!(),
                    Err(e) => {
                        eprintln!("response decode error: {e}");
                        continue;
                    }
                }
            };

            let resp_header = match resp {
                Response::Ok(response_header) => response_header,
                Response::ExtCommand(ext_command) => match ext_command {
                    ExtCommand::FileNameInvalid => continue, // remote error
                    ExtCommand::EndOfTransaction => break Ok(true),
                },
            };

            // get a reader with `payload_len` limited size
            let mut file_reader = stream.take(resp_header.payload_len() as u64);

            let mut sink_file = async || {
                // drop file payload that we don't need
                if tokio::io::copy(&mut file_reader, &mut io::sink())
                    .await
                    .is_ok()
                {
                    false
                } else {
                    eprintln!("failed to copy reader to sink");
                    true
                }
            };

            let file_name = format!("{resp_header}");
            let mut file_name_step_2_iter = file_name.as_bytes().chunks(2);

            let Some(dir_first) = file_name_step_2_iter.next() else {
                eprintln!("unexpected file_name size");
                continue;
            };
            let Some(dir_second) = file_name_step_2_iter.next() else {
                eprintln!("unexpected file_name size");
                continue;
            };

            let dir_first = str::from_utf8(dir_first).unwrap();
            let dir_second = str::from_utf8(dir_second).unwrap();

            let dir = PathBuf::new().join(dir_first).join(dir_second);

            if let Err(e) = fs::create_dir_all(&dir).await {
                eprintln!("create dir all error: {e}");
                break Ok(false);
            };

            let path = dir.join(&file_name);

            let mut file = match OpenOptions::new()
                .read(true)
                .write(true)
                .truncate(true)
                .create(true)
                .create_new(!override_files)
                .open(&path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    if matches!(e.kind(), ErrorKind::AlreadyExists) {
                        eprintln!("file exist: {file_name}");
                        if sink_file().await {
                            break Ok(false);
                        }
                        continue;
                    } else {
                        eprintln!("file open error: {e}");
                        _ = sink_file().await;
                        break Ok(false);
                    }
                }
            };

            if tokio::io::copy(&mut file_reader, &mut file).await.is_err() {
                bail!("failed to copy reader to file");
            };

            let hash = match hex::sha1sum(&mut file).await {
                Ok(hash) => hash,
                Err(e) => {
                    eprintln!("sha1sum error: {e}");
                    _ = fs::remove_file(path).await;
                    continue;
                }
            };

            if resp_header.file_hash() != hash {
                eprintln!("file hash mismatch with file_name's hash");
                _ = fs::remove_file(path).await;
                continue;
            }

            // no need to check fs_size again,
            // because the fs_size will always equals
            // to the resp_header.payload_len(),
            // and we already checked file hash
            #[cfg(false)]
            {
                let fs_size = match file.metadata().await {
                    Ok(f) => f.len(),
                    Err(e) => {
                        eprintln!("fs_size: {e}");
                        continue;
                    }
                };

                if resp_header.payload_len() as u64 != fs_size {
                    eprintln!("payload_len mismatch with fs_size");
                    continue;
                }
            }

            eprintln!("received a new file: {file_name}");
        }
    }
}
