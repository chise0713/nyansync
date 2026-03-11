use std::{
    fmt::Display,
    io::{self, Error, ErrorKind},
};

pub struct Request {
    cursor: u32,
}

impl Request {
    pub fn new(cursor: u32) -> Self {
        Self { cursor }
    }

    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    pub fn encode(&self, buf: &mut [u8; 4]) {
        *buf = self.cursor.to_be_bytes();
    }

    pub fn decode(buf: [u8; 4]) -> Self {
        Self {
            cursor: u32::from_be_bytes(buf),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Gif,
    Jpg,
    Webp,
    Png,
    EndOfTransition = 255,
}

impl FileType {
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for FileType {
    type Error = io::Error;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(FileType::Gif),
            1 => Ok(FileType::Jpg),
            2 => Ok(FileType::Webp),
            3 => Ok(FileType::Png),
            255 => Ok(FileType::EndOfTransition),
            _ => Err(Error::new(ErrorKind::InvalidData, "invalid file type")),
        }
    }
}

impl Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Gif => "gif",
                Self::Jpg => "jpg",
                Self::Webp => "wbp",
                Self::Png => "png",
                Self::EndOfTransition => "eot",
            }
        )
    }
}

pub struct Resolution {
    x: u32,
    y: u32,
}

impl Resolution {
    pub fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    pub fn encode(&self) -> [u8; 8] {
        let x_bytes = self.x.to_be_bytes();
        let y_bytes = self.y.to_be_bytes();

        [
            x_bytes[0], x_bytes[1], x_bytes[2], x_bytes[3], y_bytes[0], y_bytes[1], y_bytes[2],
            y_bytes[3],
        ]
    }

    pub fn decode(buf: [u8; 8]) -> Self {
        let x = u32::from_be_bytes(buf[0..4].try_into().unwrap());
        let y = u32::from_be_bytes(buf[4..8].try_into().unwrap());
        Self { x, y }
    }
}

impl Display for Resolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.x, self.y)
    }
}

pub struct Response {
    file_type: FileType,
    file_hash: [u8; Self::FILE_HASH_LEN],
    resolution: Resolution,
    payload_len: u32,
}

impl Response {
    pub const FILE_TYPE_LEN: usize = 1;
    pub const FILE_HASH_LEN: usize = 20;
    pub const RESOLUTION_LEN: usize = 8;
    pub const PAYLOAD_LEN_LEN: usize = 4;

    pub const TOTAL_LEN: usize =
        Self::FILE_TYPE_LEN + Self::FILE_HASH_LEN + Self::RESOLUTION_LEN + Self::PAYLOAD_LEN_LEN;

    pub fn new(
        file_type: FileType,
        file_hash: [u8; Self::FILE_HASH_LEN],
        resolution: Resolution,
        payload_len: u32,
    ) -> Self {
        Self {
            file_type,
            file_hash,
            resolution,
            payload_len,
        }
    }

    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    pub fn payload_len(&self) -> u32 {
        self.payload_len
    }

    pub fn encode(&self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.len() < Self::TOTAL_LEN {
            return Err(Error::new(ErrorKind::UnexpectedEof, "buffer too short"));
        }

        let mut n = 0;

        // file_type
        buf[n] = self.file_type.as_byte();
        n += Self::FILE_TYPE_LEN;

        // file_hash
        buf[n..n + Self::FILE_HASH_LEN].copy_from_slice(self.file_hash.as_slice());
        n += Self::FILE_HASH_LEN;

        // resolution
        buf[n..n + Self::RESOLUTION_LEN].copy_from_slice(self.resolution.encode().as_slice());
        n += Self::RESOLUTION_LEN;

        // payload_len
        buf[n..n + Self::PAYLOAD_LEN_LEN]
            .copy_from_slice(self.payload_len.to_be_bytes().as_slice());
        n += Self::PAYLOAD_LEN_LEN;

        Ok(n)
    }

    pub fn decode(buf: &[u8]) -> io::Result<Option<(Self, usize)>> {
        if buf.len() < Self::TOTAL_LEN {
            return Ok(None);
        }

        let mut n = 0;

        // file_type
        let file_type = FileType::try_from(buf[n])?;
        n += Self::FILE_TYPE_LEN;

        // file_hash
        let Ok(file_hash) = buf[n..n + Self::FILE_HASH_LEN].try_into() else {
            return Ok(None);
        };
        n += Self::FILE_HASH_LEN;

        // resolution
        let mut resolution_buf = [0u8; Self::RESOLUTION_LEN];
        resolution_buf.copy_from_slice(&buf[n..n + Self::RESOLUTION_LEN]);
        let resolution = Resolution::decode(resolution_buf);
        n += Self::RESOLUTION_LEN;

        // payload_len
        let mut payload_len_buf = [0u8; Self::PAYLOAD_LEN_LEN];
        payload_len_buf.copy_from_slice(&buf[n..n + Self::PAYLOAD_LEN_LEN]);
        let payload_len = u32::from_be_bytes(payload_len_buf);
        n += Self::PAYLOAD_LEN_LEN;

        Ok(Some((
            Self {
                file_type,
                file_hash,
                resolution,
                payload_len,
            },
            n,
        )))
    }
}

impl Display for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}",
            str::from_utf8(hex::bytes_to_hex(&self.file_hash).as_slice()).unwrap(),
            self.payload_len,
            self.resolution,
            self.file_type
        )
    }
}

pub type ResponsePayload = Box<[u8]>;

pub mod hex {
    use super::*;

    const HEX_LEN: usize = Response::FILE_HASH_LEN * 2;
    const BYTES_LEN: usize = Response::FILE_HASH_LEN;

    pub const fn hex_to_bytes(hex: &[u8; HEX_LEN]) -> [u8; BYTES_LEN] {
        let mut out = [0u8; BYTES_LEN];
        let mut i = 0;
        while i < BYTES_LEN {
            let hi = match hex[i * 2] {
                b'0'..=b'9' => hex[i * 2] - b'0',
                b'a'..=b'f' => hex[i * 2] - b'a' + 10,
                b'A'..=b'F' => hex[i * 2] - b'A' + 10,
                _ => panic!("invalid hex"),
            };
            let lo = match hex[i * 2 + 1] {
                b'0'..=b'9' => hex[i * 2 + 1] - b'0',
                b'a'..=b'f' => hex[i * 2 + 1] - b'a' + 10,
                b'A'..=b'F' => hex[i * 2 + 1] - b'A' + 10,
                _ => panic!("invalid hex"),
            };
            out[i] = (hi << 4) | lo;
            i += 1;
        }
        out
    }

    pub const fn bytes_to_hex(bytes: &[u8; BYTES_LEN]) -> [u8; HEX_LEN] {
        const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

        let mut out = [0u8; HEX_LEN];
        let mut i = 0;

        while i < BYTES_LEN {
            let b = bytes[i];
            out[i * 2] = HEX_CHARS[(b >> 4) as usize];
            out[i * 2 + 1] = HEX_CHARS[(b & 0x0F) as usize];
            i += 1;
        }

        out
    }
}
