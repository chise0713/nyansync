use std::{
    ffi::OsStr,
    io::{self, Error, ErrorKind},
    path::Path,
};

#[derive(Debug)]
pub struct PathTable {
    inner: Box<[u8]>,
    offsets: Box<[u32]>,
}

impl PathTable {
    pub fn new<I, P>(paths: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let path_iter = paths.into_iter();
        let (capacity, _) = path_iter.size_hint();

        let mut buf = Vec::new();
        let mut offsets = Vec::with_capacity(capacity);

        path_iter
            .scan(true, |first, path| {
                let is_first = *first;
                *first = false;
                Some((is_first, path))
            })
            .try_for_each(|(is_first, path)| -> io::Result<()> {
                let path = path.as_ref();

                if !path.is_absolute() {
                    return Err(Error::new(ErrorKind::InvalidInput, "path is not absolute"));
                }

                let bytes = path.as_os_str().as_encoded_bytes();

                const NUL: u8 = 0;
                if bytes.contains(&NUL) {
                    return Err(Error::new(ErrorKind::InvalidData, "path contains NUL"));
                }

                if !is_first {
                    buf.push(b'\0');
                }

                if buf.len() > u32::MAX as usize {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "path table exceeds 4 GiB limit",
                    ));
                }
                offsets.push(buf.len() as u32);

                buf.extend_from_slice(bytes);

                Ok(())
            })?;

        Ok(Self {
            inner: buf.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
        })
    }

    pub fn get(&self, i: usize) -> Option<&Path> {
        let start = *self.offsets.get(i)? as usize;

        let end = if i + 1 < self.offsets.len() {
            (self.offsets[i + 1] - 1) as usize
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
    use super::*;

    #[test]
    fn test_single_path() {
        let ft = PathTable::new(["/a"]).unwrap();

        assert_eq!(ft.offsets.len(), 1);
        assert_eq!(ft.get(0).unwrap(), Path::new("/a"));
    }

    #[test]
    fn test_multiple_paths() {
        let ft = PathTable::new(["/a", "/b", "/c"]).unwrap();

        assert_eq!(ft.offsets.len(), 3);

        assert_eq!(ft.get(0).unwrap(), Path::new("/a"));
        assert_eq!(ft.get(1).unwrap(), Path::new("/b"));
        assert_eq!(ft.get(2).unwrap(), Path::new("/c"));
    }

    #[test]
    fn test_long_paths() {
        let ft = PathTable::new(["/aaa", "/bbbb", "/ccccc"]).unwrap();

        assert_eq!(ft.get(0).unwrap(), Path::new("/aaa"));
        assert_eq!(ft.get(1).unwrap(), Path::new("/bbbb"));
        assert_eq!(ft.get(2).unwrap(), Path::new("/ccccc"));
    }

    #[test]
    fn test_invalid_relative_path() {
        let err = PathTable::new(["a"]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_out_of_bounds() {
        let ft = PathTable::new(["/a", "/b"]).unwrap();

        assert!(ft.get(2).is_none());
        assert!(ft.get(100).is_none());
    }

    #[test]
    fn test_empty() {
        let ft = PathTable::new(&[] as &[&str]).unwrap();

        assert_eq!(ft.offsets.len(), 0);
        assert!(ft.get(0).is_none());
    }

    #[test]
    fn test_internal_layout() {
        let ft = PathTable::new(["/a", "/b"]).unwrap();

        // "/a\0/b"
        assert_eq!(ft.inner.as_ref(), b"/a\0/b");

        // offsets: [0, 3]
        assert_eq!(ft.offsets.as_ref(), &[0, 3]);
    }

    #[test]
    fn test_iter_equivalence() {
        let paths = ["/a", "/b", "/c"];
        let ft = PathTable::new(paths).unwrap();

        let collected: Box<[_]> = (0..ft.offsets.len()).map(|i| ft.get(i).unwrap()).collect();

        let expected: Box<[_]> = paths.iter().map(Path::new).collect();

        assert_eq!(collected, expected);
    }
}
