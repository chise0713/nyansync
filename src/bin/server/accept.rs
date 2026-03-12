use std::{collections::BTreeSet, path::Path, sync::Arc};

use nyansync::{FileType, Request, Resolution, Response};
use tokio::{
    fs::File,
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::broadcast::{Receiver, Sender},
};

pub struct Accept;

impl Accept {
    pub async fn accept(
        ln: Arc<TcpListener>,
        set: Arc<BTreeSet<Box<Path>>>,
        notify_shutdown: Sender<()>,
        mut shutdown: Receiver<()>,
    ) {
        loop {
            let conn = tokio::select! {
                conn = ln.accept() => {
                    conn
                },
                _ = shutdown.recv() => {
                    return;
                }
            };
            let (stream, addr) = match conn {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("accept error: {e}");
                    // notify all other conn awaiter
                    _ = notify_shutdown.send(());
                    return;
                }
            };

            eprintln!("new client: {addr}");

            tokio::spawn(Self::stream_handle(stream, set.clone()));
        }
    }

    pub async fn stream_handle(mut stream: TcpStream, set: Arc<BTreeSet<Box<Path>>>) {
        let mut buf = 0u32.to_be_bytes();

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

            let resp_bytes: &mut [u8] = &mut [0; Response::TOTAL_LEN];
            let Some(path) = set.iter().nth(cursor as usize) else {
                _ = stream
                    .write_all(&[FileType::EndOfTransaction.as_byte()])
                    .await;
                break;
            };
            let fs_size = match path.metadata() {
                Ok(f) => f.len(),
                Err(e) => {
                    eprintln!("fs_size: {e}");
                    break;
                }
            };

            let file_name_invalid = async {
                _ = stream
                    .write_all(&[FileType::FileNameInvalid.as_byte()])
                    .await;
            };

            let file_name = path.file_name().unwrap();
            let Some(file_name) = file_name.to_str() else {
                eprintln!("file_name is not a valid utf-8");
                file_name_invalid.await;
                return;
            };

            let mut split = file_name.split('-');
            let Some(hex) = split.next() else {
                eprintln!("file name: no hex");
                file_name_invalid.await;
                break;
            };
            let Some(file_size) = split.next() else {
                eprintln!("file name: no file_size");
                file_name_invalid.await;
                break;
            };
            let Some(x) = split.next() else {
                eprintln!("file name: no res_x");
                file_name_invalid.await;
                break;
            };
            let Some(y) = split.next() else {
                eprintln!("file name: no res_y");
                file_name_invalid.await;
                break;
            };
            let Some(typ) = split.next() else {
                eprintln!("file name: no typ");
                file_name_invalid.await;
                break;
            };

            if split.next().is_some() {
                eprintln!("file name field exceed 5");
                file_name_invalid.await;
                break;
            }

            let Ok(hex) = hex.as_bytes().try_into() else {
                eprintln!("hex length not equals to 40");
                file_name_invalid.await;
                break;
            };

            let hash = nyansync::hex::hex_to_bytes(&hex);
            let Ok(file_size) = file_size.parse() else {
                eprintln!("file_size not a valid u32");
                file_name_invalid.await;
                break;
            };
            if fs_size != file_size as u64 {
                eprintln!("fs_size not equals to file_size");
                file_name_invalid.await;
                break;
            }
            let Ok(x) = x.parse() else {
                eprintln!("res_x not a valid u32");
                file_name_invalid.await;
                break;
            };
            let Ok(y) = y.parse() else {
                eprintln!("res_y not a valid u32");
                file_name_invalid.await;
                break;
            };
            let typ = match typ {
                "gif" => FileType::Gif,
                "jpg" => FileType::Jpg,
                "wbp" => FileType::Webp,
                "png" => FileType::Png,
                _ => {
                    eprintln!("unknown typ: {typ}");
                    file_name_invalid.await;
                    break;
                }
            };

            let resp = Response::new(typ, hash, Resolution::new(x, y), file_size);
            if resp.encode(resp_bytes).is_err() {
                eprintln!("resp encode failed");
                break;
            };

            if stream.write_all(resp_bytes).await.is_err() {
                eprintln!("write_all resp_bytes failed");
                break;
            };

            let mut file = match File::open(path).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("failed to open file `{}`: {e}", path.display());
                    break;
                }
            };

            if tokio::io::copy(&mut file, &mut stream).await.is_err() {
                eprintln!("copy file to stream failed");
                break;
            };
        }
    }
}
