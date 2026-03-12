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
}

impl FileType {
    pub const fn as_byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for FileType {
    type Error = io::Error;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Gif),
            1 => Ok(Self::Jpg),
            2 => Ok(Self::Webp),
            3 => Ok(Self::Png),
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
        let mut out = [0; 8];
        out[..4].copy_from_slice(&self.x.to_be_bytes());
        out[4..].copy_from_slice(&self.y.to_be_bytes());
        out
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

pub enum Response {
    Ok(ResponseHeader),
    ExtCommand(ExtCommand),
}

impl Response {
    pub fn encode(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n = 0;

        match self {
            Self::Ok(response_header) => {
                if buf.len() < ResponseHeader::TOTAL_LEN {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "buffer too short"));
                }

                // file_type
                buf[n] = response_header.file_type.as_byte();
                n += ResponseHeader::FILE_TYPE_LEN;

                // file_hash
                buf[n..n + ResponseHeader::FILE_HASH_LEN]
                    .copy_from_slice(response_header.file_hash.as_slice());
                n += ResponseHeader::FILE_HASH_LEN;

                // resolution
                buf[n..n + ResponseHeader::RESOLUTION_LEN]
                    .copy_from_slice(response_header.resolution.encode().as_slice());
                n += ResponseHeader::RESOLUTION_LEN;

                // payload_len
                buf[n..n + ResponseHeader::PAYLOAD_LEN_LEN]
                    .copy_from_slice(response_header.payload_len.to_be_bytes().as_slice());
                n += ResponseHeader::PAYLOAD_LEN_LEN;
            }
            Self::ExtCommand(ext_command) => {
                buf[n] = ext_command.as_byte();
                n += 1;
            }
        }

        Ok(n)
    }

    pub fn decode(buf: &[u8]) -> io::Result<Option<(Self, usize)>> {
        // len is 0
        if buf.is_empty() {
            return Ok(None);
        }

        let mut n = 0;

        let first_byte = buf[n];

        if first_byte > 127 {
            return Ok(Some((
                Response::ExtCommand(ExtCommand::try_from(first_byte)?),
                1,
            )));
        }

        if buf.len() < ResponseHeader::TOTAL_LEN {
            return Ok(None);
        }

        // file_type
        let file_type = FileType::try_from(first_byte)?;
        n += ResponseHeader::FILE_TYPE_LEN;

        // file_hash
        let mut file_hash = [0u8; ResponseHeader::FILE_HASH_LEN];
        file_hash.copy_from_slice(&buf[n..n + ResponseHeader::FILE_HASH_LEN]);
        n += ResponseHeader::FILE_HASH_LEN;

        // resolution
        let mut resolution_buf = [0u8; ResponseHeader::RESOLUTION_LEN];
        resolution_buf.copy_from_slice(&buf[n..n + ResponseHeader::RESOLUTION_LEN]);
        let resolution = Resolution::decode(resolution_buf);
        n += ResponseHeader::RESOLUTION_LEN;

        // payload_len
        let mut payload_len_buf = [0u8; ResponseHeader::PAYLOAD_LEN_LEN];
        payload_len_buf.copy_from_slice(&buf[n..n + ResponseHeader::PAYLOAD_LEN_LEN]);
        let payload_len = u32::from_be_bytes(payload_len_buf);
        n += ResponseHeader::PAYLOAD_LEN_LEN;

        Ok(Some((
            Self::Ok(ResponseHeader {
                file_type,
                file_hash,
                resolution,
                payload_len,
            }),
            n,
        )))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ParseFileNameError {
    #[error("field missing")]
    FieldMissing,
    #[error("field exceed")]
    FieldExceed,
    #[error("invalid hex")]
    InvalidHex,
    #[error("invalid file size")]
    InvalidFileSize,
    #[error("invalid resolution.x")]
    InvalidResX,
    #[error("invalid resolution.y")]
    InvalidResY,
    #[error("unknown type")]
    UnknownType,
}

impl TryFrom<&str> for ResponseHeader {
    type Error = ParseFileNameError;

    fn try_from(file_name: &str) -> Result<Self, Self::Error> {
        let mut split = file_name.split('-');

        let hex = split.next().ok_or(ParseFileNameError::FieldMissing)?;
        let file_size = split.next().ok_or(ParseFileNameError::FieldMissing)?;
        let x = split.next().ok_or(ParseFileNameError::FieldMissing)?;
        let y = split.next().ok_or(ParseFileNameError::FieldMissing)?;
        let typ = split.next().ok_or(ParseFileNameError::FieldMissing)?;

        if split.next().is_some() {
            return Err(ParseFileNameError::FieldExceed);
        }

        let hex: [u8; 40] = hex
            .as_bytes()
            .try_into()
            .map_err(|_| ParseFileNameError::InvalidHex)?;

        let hash = hex::hex_to_bytes(&hex).ok_or(ParseFileNameError::InvalidHex)?;

        let file_size: u32 = file_size
            .parse()
            .map_err(|_| ParseFileNameError::InvalidFileSize)?;

        let x: u32 = x.parse().map_err(|_| ParseFileNameError::InvalidResX)?;
        let y: u32 = y.parse().map_err(|_| ParseFileNameError::InvalidResY)?;

        let typ = match typ {
            "gif" => FileType::Gif,
            "jpg" => FileType::Jpg,
            "wbp" => FileType::Webp,
            "png" => FileType::Png,
            _ => return Err(ParseFileNameError::UnknownType),
        };

        Ok(ResponseHeader::new(
            typ,
            hash,
            Resolution::new(x, y),
            file_size,
        ))
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum ExtCommand {
    FileNameInvalid = 254,
    EndOfTransaction = 255,
}

impl ExtCommand {
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for ExtCommand {
    type Error = io::Error;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            254 => Ok(Self::FileNameInvalid),
            255 => Ok(Self::EndOfTransaction),
            _ => Err(Error::new(ErrorKind::InvalidData, "invalid ext command")),
        }
    }
}

pub struct ResponseHeader {
    file_type: FileType,
    file_hash: [u8; Self::FILE_HASH_LEN],
    resolution: Resolution,
    payload_len: u32,
}

impl ResponseHeader {
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
}

impl Display for ResponseHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}",
            // won't fail so unwrap
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

    const HEX_LEN: usize = ResponseHeader::FILE_HASH_LEN * 2;
    const BYTES_LEN: usize = ResponseHeader::FILE_HASH_LEN;

    pub const fn hex_to_bytes(hex: &[u8; HEX_LEN]) -> Option<[u8; BYTES_LEN]> {
        let mut out = [0u8; BYTES_LEN];
        let mut i = 0;
        while i < BYTES_LEN {
            let hi = match hex[i * 2] {
                b'0'..=b'9' => hex[i * 2] - b'0',
                b'a'..=b'f' => hex[i * 2] - b'a' + 10,
                b'A'..=b'F' => hex[i * 2] - b'A' + 10,
                _ => return None,
            };
            let lo = match hex[i * 2 + 1] {
                b'0'..=b'9' => hex[i * 2 + 1] - b'0',
                b'a'..=b'f' => hex[i * 2 + 1] - b'a' + 10,
                b'A'..=b'F' => hex[i * 2 + 1] - b'A' + 10,
                _ => return None,
            };
            out[i] = (hi << 4) | lo;
            i += 1;
        }
        Some(out)
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
