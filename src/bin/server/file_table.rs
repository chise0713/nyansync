use std::{
    ffi::OsStr,
    io::{self, Error, ErrorKind},
    path::Path,
};

#[derive(Debug)]
pub struct FileTable {
    inner: Box<[u8]>,
    offsets: Box<[usize]>,
}

impl FileTable {
    pub fn new<I, P>(paths: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut buf = Vec::new();
        let mut offsets = Vec::new();

        for (i, path) in paths.into_iter().enumerate() {
            let path = path.as_ref();

            if !path.is_absolute() {
                return Err(Error::new(ErrorKind::InvalidInput, "path is not absolute"));
            }

            const NUL: u8 = 0;
            if path.as_os_str().as_encoded_bytes().contains(&NUL) {
                return Err(Error::new(ErrorKind::InvalidData, "path contains NUL"));
            }

            if i != 0 {
                buf.push(b'\0');
            }
            offsets.push(buf.len());

            buf.extend_from_slice(path.as_os_str().as_encoded_bytes());
        }

        Ok(Self {
            inner: buf.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
        })
    }

    pub fn get(&self, i: usize) -> Option<&Path> {
        let start = *self.offsets.get(i)?;

        let end = if i + 1 < self.offsets.len() {
            self.offsets[i + 1] - 1
        } else {
            self.inner.len()
        };

        let slice = &self.inner[start..end];

        // # Safety
        // - `slice` originates from `Path::as_os_str().as_encoded_bytes()`
        // - `inner` is only constructed in `Self::new`
        // - no mutation occurs, so encoding remains valid
        let os = unsafe { OsStr::from_encoded_bytes_unchecked(slice) };
        Some(Path::new(os))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_single_path() {
        let ft = FileTable::new(["/a"]).unwrap();

        assert_eq!(ft.offsets.len(), 1);
        assert_eq!(ft.get(0).unwrap(), Path::new("/a"));
    }

    #[test]
    fn test_multiple_paths() {
        let ft = FileTable::new(["/a", "/b", "/c"]).unwrap();

        assert_eq!(ft.offsets.len(), 3);

        assert_eq!(ft.get(0).unwrap(), Path::new("/a"));
        assert_eq!(ft.get(1).unwrap(), Path::new("/b"));
        assert_eq!(ft.get(2).unwrap(), Path::new("/c"));
    }

    #[test]
    fn test_long_paths() {
        let ft = FileTable::new(["/aaa", "/bbbb", "/ccccc"]).unwrap();

        assert_eq!(ft.get(0).unwrap(), Path::new("/aaa"));
        assert_eq!(ft.get(1).unwrap(), Path::new("/bbbb"));
        assert_eq!(ft.get(2).unwrap(), Path::new("/ccccc"));
    }

    #[test]
    fn test_invalid_relative_path() {
        let err = FileTable::new(["a"]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_out_of_bounds() {
        let ft = FileTable::new(["/a", "/b"]).unwrap();

        assert!(ft.get(2).is_none());
        assert!(ft.get(100).is_none());
    }

    #[test]
    fn test_empty() {
        let ft = FileTable::new(std::iter::empty::<&str>()).unwrap();

        assert_eq!(ft.offsets.len(), 0);
        assert!(ft.get(0).is_none());
    }

    #[test]
    fn test_internal_layout() {
        let ft = FileTable::new(["/a", "/b"]).unwrap();

        // "/a\0/b"
        assert_eq!(ft.inner.as_ref(), b"/a\0/b");

        // offsets: [0, 3]
        assert_eq!(ft.offsets.as_ref(), &[0, 3]);
    }

    #[test]
    fn test_iter_equivalence() {
        let paths = ["/a", "/b", "/c"];
        let ft = FileTable::new(paths).unwrap();

        let collected: Vec<_> = (0..ft.offsets.len()).map(|i| ft.get(i).unwrap()).collect();

        let expected: Vec<_> = paths.iter().map(Path::new).collect();

        assert_eq!(collected, expected);
    }
}
