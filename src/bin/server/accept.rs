use std::{io::SeekFrom, path::Path, sync::Arc};

use nyansync::{ExtCommand, Request, Response, ResponseHeader};
use sha1::{Digest as _, Sha1};
use tokio::{
    fs::File,
    io::{self, AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

pub struct Accept;

impl Accept {
    pub async fn accept(mut stream: TcpStream, files: Arc<Box<[Box<Path>]>>) {
        let mut buf = 0u32.to_be_bytes();
        let resp_bytes: &mut [u8] = &mut [0; ResponseHeader::TOTAL_LEN];

        loop {
            match stream.read_exact(buf.as_mut_slice()).await {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("read_exact error: {e}");
                    break;
                }
            };

            let req = Request::decode(buf);

            let cursor = req.cursor();

            let Some(path) = files.get(cursor as usize) else {
                let mut buf = [0];
                let resp = Response::ExtCommand(ExtCommand::EndOfTransaction);
                if resp.encode(&mut buf).is_err() {
                    break;
                };
                _ = stream.write_all(&buf).await;
                break;
            };

            let mut file = match File::open(path).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("failed to open file `{}`: {e}", path.display());
                    break;
                }
            };

            let hash = match sha1sum(&mut file).await {
                Ok(hash) => hash,
                Err(e) => {
                    eprintln!("sha1sum error: {e}");
                    return;
                }
            };

            let fs_size = match path.metadata() {
                Ok(f) => f.len(),
                Err(e) => {
                    eprintln!("fs_size: {e}");
                    break;
                }
            };

            let file_name_invalid = async {
                let mut buf = [0];
                let resp = Response::ExtCommand(ExtCommand::FileNameInvalid);
                if resp.encode(&mut buf).is_err() {
                    return;
                };
                _ = stream.write_all(&buf).await;
            };

            let file_name = path.file_name().unwrap();
            let Some(file_name) = file_name.to_str() else {
                eprintln!("file_name is not a valid utf-8");
                file_name_invalid.await;
                break;
            };

            let header = match ResponseHeader::try_from(file_name) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("file name parse error: {e}");
                    file_name_invalid.await;
                    break;
                }
            };

            if header.payload_len() as u64 != fs_size {
                eprintln!("file_name's size mismatch with fs_size");
                file_name_invalid.await;
                break;
            }

            if header.file_hash() != hash {
                eprintln!("file_name's hash mismatch with file hash");
                file_name_invalid.await;
                break;
            }

            let resp = Response::Ok(header);
            if resp.encode(resp_bytes).is_err() {
                eprintln!("resp encode failed");
                break;
            };

            if stream.write_all(resp_bytes).await.is_err() {
                eprintln!("write_all resp_bytes failed");
                break;
            };

            if tokio::io::copy(&mut file, &mut stream).await.is_err() {
                eprintln!("copy file to stream failed");
                break;
            };
        }
    }
}

async fn sha1sum(file: &mut File) -> io::Result<[u8; 20]> {
    let mut hasher = Sha1::new();

    let mut buffer = [0u8; 32 * 1024];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    file.seek(SeekFrom::Start(0)).await?;

    Ok(*hasher.finalize().as_ref())
}
