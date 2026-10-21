//! File access. Replaces `loci.common.RandomAccessInputStream`.
//!
//! Readers read with positional I/O (`pread`), so one open file can serve
//! many threads at once without a shared cursor.

use std::fs::File;
use std::path::Path;

use crate::error::{Error, Result};

/// A read-only file that supports concurrent positional reads.
#[derive(Debug)]
pub struct RandomAccessFile {
    file: File,
    len: u64,
}

impl RandomAccessFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(RandomAccessFile { file, len })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Fills `buf` from `offset`, failing if the file is too short.
    pub fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(Error::Format(format!(
                "read of {} bytes at offset {offset} runs past end of file ({} bytes)",
                buf.len(),
                self.len
            )));
        }
        read_exact_at(&self.file, buf, offset)?;
        Ok(())
    }

    pub fn read_vec_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0; len];
        self.read_exact_at(&mut buf, offset)?;
        Ok(buf)
    }

    /// Reads up to `len` bytes, returning fewer if the file ends first.
    pub fn read_prefix(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let available = self.len.saturating_sub(offset).min(len as u64) as usize;
        self.read_vec_at(offset, available)
    }

    /// A sequential cursor over the file, for parsing headers.
    pub fn cursor(&self, offset: u64, little_endian: bool) -> Cursor<'_> {
        Cursor {
            file: self,
            pos: offset,
            little_endian,
        }
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A sequential reader over a [`RandomAccessFile`] with a chosen byte order.
pub struct Cursor<'a> {
    file: &'a RandomAccessFile,
    pub pos: u64,
    pub little_endian: bool,
}

macro_rules! read_num {
    ($name:ident, $t:ty) => {
        pub fn $name(&mut self) -> Result<$t> {
            let mut b = [0u8; std::mem::size_of::<$t>()];
            self.read_exact(&mut b)?;
            Ok(if self.little_endian {
                <$t>::from_le_bytes(b)
            } else {
                <$t>::from_be_bytes(b)
            })
        }
    };
}

impl Cursor<'_> {
    pub fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.file.read_exact_at(buf, self.pos)?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    pub fn read_vec(&mut self, len: usize) -> Result<Vec<u8>> {
        let v = self.file.read_vec_at(self.pos, len)?;
        self.pos += len as u64;
        Ok(v)
    }

    pub fn skip(&mut self, n: u64) {
        self.pos += n;
    }

    pub fn remaining(&self) -> u64 {
        self.file.len().saturating_sub(self.pos)
    }

    read_num!(read_u8, u8);
    read_num!(read_u16, u16);
    read_num!(read_u32, u32);
    read_num!(read_i32, i32);
    read_num!(read_u64, u64);
    read_num!(read_i64, i64);

    /// Reads `n` UTF-16 code units in the cursor's byte order. Equivalent to
    /// calling Java's `DataInput.readChar` `n` times.
    pub fn read_utf16(&mut self, n: usize) -> Result<String> {
        let bytes = self.read_vec(n * 2)?;
        let units = bytes.chunks_exact(2).map(|c| {
            let pair = [c[0], c[1]];
            if self.little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        });
        Ok(char::decode_utf16(units)
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect())
    }
}
