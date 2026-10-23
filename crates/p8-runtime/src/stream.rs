//! Resources, and the one kind there is so far: streams (files, the
//! standard streams, php://memory and php://temp).
//!
//! A resource is a Value::Object of a resource class (CLASS_RESOURCE),
//! numbered by its own counter as PHP numbers resources, holding a
//! [`Resource`] as its data.
//!
//! Reference: $PHP_SRC_PATH/main/streams/streams.c,
//! $PHP_SRC_PATH/main/streams/plain_wrapper.c, $PHP_SRC_PATH/ext/standard/file.c

use crate::ext::Ctx;
use crate::object::Object;
use crate::value::Value;
use std::cell::{Cell, RefCell};
use std::io::{Read, Seek, SeekFrom, Write};

/// What a resource object holds.
pub struct Resource {
    /// get_resource_type(): "stream", ...
    pub kind: &'static str,
    pub closed: Cell<bool>,
    pub stream: RefCell<Stream>,
}

/// The resource a value is, if it is one.
pub fn resource_of(v: &Value) -> Option<std::rc::Rc<Object>> {
    match v.deref() {
        Value::Object(o) if o.class.flags & crate::object::CLASS_RESOURCE != 0 => Some(o),
        _ => None,
    }
}

pub fn resource(o: &Object) -> Option<&Resource> {
    o.data.as_ref().and_then(|d| d.0.downcast_ref::<Resource>())
}

pub enum Target {
    File(std::fs::File),
    /// php://memory, php://temp, tmpfile()'s contents.
    Memory(Vec<u8>),
    Stdin,
    /// What the host shows (bypassing output buffers, as PHP's STDOUT).
    Stdout,
    Stderr,
}

pub struct Stream {
    pub target: Target,
    pub path: String,
    pub mode: String,
    /// Read or write position (Memory; Files keep their own).
    pub pos: usize,
    pub eof: bool,
    /// Read ahead of what the script has taken (fgets reads past the line).
    pub buffer: Vec<u8>,
}

impl Stream {
    pub fn new(target: Target, path: &str, mode: &str) -> Stream {
        Stream { target, path: path.to_string(), mode: mode.to_string(), pos: 0, eof: false, buffer: Vec::new() }
    }

    pub fn readable(&self) -> bool {
        self.mode.contains('r') || self.mode.contains('+')
    }

    pub fn writable(&self) -> bool {
        self.mode.contains(['w', 'a', 'x', 'c', '+'])
    }

    /// Up to `n` bytes from the underlying target (not the buffer).
    fn fill(&mut self, n: usize) -> Vec<u8> {
        match &mut self.target {
            Target::File(f) => {
                let mut buf = vec![0; n];
                let got = f.read(&mut buf).unwrap_or(0);
                buf.truncate(got);
                buf
            }
            Target::Memory(data) => {
                let end = (self.pos + n).min(data.len());
                let out = data[self.pos.min(end)..end].to_vec();
                self.pos = end;
                out
            }
            Target::Stdin => {
                let mut buf = vec![0; n];
                let got = std::io::stdin().read(&mut buf).unwrap_or(0);
                buf.truncate(got);
                buf
            }
            Target::Stdout | Target::Stderr => Vec::new(),
        }
    }

    /// fread: up to `n` bytes.
    pub fn read(&mut self, n: usize) -> Vec<u8> {
        let mut out: Vec<u8> = self.buffer.drain(..self.buffer.len().min(n)).collect();
        if out.len() < n {
            let want = n - out.len();
            let more = self.fill(want);
            // A short read is the end (plain files and memory).
            if more.len() < want {
                self.eof = true;
            }
            out.extend(more);
        }
        out
    }

    /// fgets: through the next newline (or `limit - 1` bytes), None at the end.
    pub fn gets(&mut self, limit: Option<usize>) -> Option<Vec<u8>> {
        let max = limit.map(|l| l.saturating_sub(1)).unwrap_or(usize::MAX);
        loop {
            if let Some(at) = self.buffer.iter().position(|&b| b == b'\n')
                && at < max
            {
                return Some(self.buffer.drain(..=at).collect());
            }
            if self.buffer.len() >= max {
                return Some(self.buffer.drain(..max).collect());
            }
            let more = self.fill(8192);
            if more.len() < 8192 {
                self.eof = true;
            }
            if more.is_empty() {
                if self.buffer.is_empty() {
                    return None;
                }
                return Some(std::mem::take(&mut self.buffer));
            }
            self.buffer.extend(more);
        }
    }

    /// Everything left.
    pub fn read_all(&mut self) -> Vec<u8> {
        let mut out = std::mem::take(&mut self.buffer);
        loop {
            let more = self.fill(65536);
            if more.is_empty() {
                break;
            }
            out.extend(more);
        }
        self.eof = true;
        out
    }

    pub fn write(&mut self, ctx: &mut dyn Ctx, bytes: &[u8]) -> Option<usize> {
        // Writing after reading ahead: back to where the script is.
        if !self.buffer.is_empty() {
            let back = self.buffer.len();
            self.buffer.clear();
            match &mut self.target {
                Target::File(f) => {
                    let _ = f.seek(SeekFrom::Current(-(back as i64)));
                }
                Target::Memory(_) => self.pos -= back,
                _ => {}
            }
        }
        let append = self.mode.starts_with('a');
        match &mut self.target {
            Target::File(f) => f.write_all(bytes).ok().map(|()| bytes.len()),
            Target::Memory(data) => {
                if append {
                    self.pos = data.len();
                }
                let end = self.pos + bytes.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[self.pos..end].copy_from_slice(bytes);
                self.pos = end;
                Some(bytes.len())
            }
            Target::Stdout => {
                ctx.write_stdout(bytes);
                Some(bytes.len())
            }
            Target::Stderr => {
                let _ = std::io::stderr().write_all(bytes);
                Some(bytes.len())
            }
            Target::Stdin => None,
        }
    }

    pub fn tell(&mut self) -> i64 {
        let ahead = self.buffer.len() as i64;
        match &mut self.target {
            Target::File(f) => f.stream_position().map(|p| p as i64 - ahead).unwrap_or(0),
            Target::Memory(_) => self.pos as i64 - ahead,
            _ => 0,
        }
    }

    /// fseek (whence: SEEK_SET 0, SEEK_CUR 1, SEEK_END 2): whether it moved.
    pub fn seek(&mut self, offset: i64, whence: i64) -> bool {
        let here = self.tell();
        self.buffer.clear();
        self.eof = false;
        match &mut self.target {
            Target::File(f) => {
                let to = match whence {
                    1 => SeekFrom::Start((here + offset).max(0) as u64),
                    2 => SeekFrom::End(offset),
                    _ => SeekFrom::Start(offset.max(0) as u64),
                };
                f.seek(to).is_ok()
            }
            Target::Memory(data) => {
                let to = match whence {
                    1 => here + offset,
                    2 => data.len() as i64 + offset,
                    _ => offset,
                };
                if to < 0 {
                    return false;
                }
                self.pos = to as usize;
                true
            }
            _ => false,
        }
    }

    pub fn truncate(&mut self, size: usize) -> bool {
        match &mut self.target {
            Target::File(f) => f.set_len(size as u64).is_ok(),
            Target::Memory(data) => {
                data.resize(size, 0);
                true
            }
            _ => false,
        }
    }

    pub fn flush(&mut self) -> bool {
        match &mut self.target {
            Target::File(f) => f.flush().is_ok(),
            _ => true,
        }
    }

    /// feof: whether a read hit the end (and nothing is buffered).
    pub fn at_end(&self) -> bool {
        self.eof && self.buffer.is_empty()
    }
}
