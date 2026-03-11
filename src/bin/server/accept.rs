use std::{collections::BTreeSet, path::Path, sync::Arc};

use nyansync::{FileType, Request, Resolution, Response};
use tokio::{
    fs::File,
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::Semaphore,
};

pub struct Accept;

impl Accept {
    pub async fn accept(
        ln: Arc<TcpListener>,
        set: Arc<BTreeSet<Box<Path>>>,
        semaphore: Arc<Semaphore>,
    ) {
        loop {
            let (mut stream, addr) = match ln.accept().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("accept error: {e}");
                    return;
                }
            };

            eprintln!("new client: {addr}");

            let _permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("acquire_owned error: {e}");
                    return;
                }
            };

            let set = set.clone();

            tokio::spawn(async move {
                let mut buf = 0u32.to_be_bytes();

                match stream.read_exact(buf.as_mut_slice()).await {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("read_exact error: {e}");
                        return;
                    }
                };

                let req = Request::decode(buf);

                let cursor = req.cursor();

                let mut resp_bytes = vec![0; Response::TOTAL_LEN];
                let Some(path) = set.iter().nth(cursor as usize) else {
                    _ = stream
                        .write_all(&[FileType::EndOfTransition.as_byte()])
                        .await;
                    return;
                };

                let file_name = path.file_name().unwrap();
                let file_name = file_name.to_string_lossy();

                let not_valid = async {
                    _ = stream.write_all(&[FileType::NotValid.as_byte()]).await;
                };

                let mut split = file_name.split('-');
                let Some(hex) = split.next() else {
                    eprintln!("file name: no hex");
                    not_valid.await;
                    return;
                };
                let Some(file_size) = split.next() else {
                    eprintln!("file name: no file_size");
                    not_valid.await;
                    return;
                };
                let Some(x) = split.next() else {
                    eprintln!("file name: no res_x");
                    not_valid.await;
                    return;
                };
                let Some(y) = split.next() else {
                    eprintln!("file name: no res_y");
                    not_valid.await;
                    return;
                };
                let Some(typ) = split.next() else {
                    eprintln!("file name: no typ");
                    not_valid.await;
                    return;
                };

                if split.next().is_some() {
                    eprintln!("file name field exceed 5");
                    not_valid.await;
                    return;
                }

                let Ok(hex) = hex.as_bytes().try_into() else {
                    eprintln!("hex length not equals to 40");
                    not_valid.await;
                    return;
                };

                let hash = nyansync::hex::hex_to_bytes(&hex);
                let Ok(file_size) = file_size.parse() else {
                    eprintln!("file_size not a valid u32");
                    not_valid.await;
                    return;
                };
                let Ok(x) = x.parse() else {
                    eprintln!("res_x not a valid u32");
                    not_valid.await;
                    return;
                };
                let Ok(y) = y.parse() else {
                    eprintln!("res_y not a valid u32");
                    not_valid.await;
                    return;
                };
                let typ = match typ {
                    "gif" => FileType::Gif,
                    "jpg" => FileType::Jpg,
                    "wbp" => FileType::Webp,
                    "png" => FileType::Png,
                    _ => {
                        eprintln!("unknown typ: {typ}");
                        not_valid.await;
                        return;
                    }
                };

                let resp = Response::new(typ, hash, Resolution::new(x, y), file_size);
                if resp.encode(&mut resp_bytes).is_err() {
                    eprintln!("resp encode failed");
                    return;
                };

                if stream.write_all(&resp_bytes).await.is_err() {
                    eprintln!("write_all resp_bytes failed");
                    return;
                };

                let mut file = match File::open(path).await {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("failed to open file `{}`: {e}", path.display());
                        return;
                    }
                };

                if tokio::io::copy(&mut file, &mut stream).await.is_err() {
                    eprintln!("copy file to stream failed");
                    return;
                };

                _ = _permit;
            });
        }
    }
}
