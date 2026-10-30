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

/// The host's MAXPATHLEN (PHP_MAXPATHLEN): realpath() and friends fail
/// on a path this long or longer.
pub const MAXPATHLEN: usize = if cfg!(target_os = "macos") { 1024 } else { 4096 };

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
    /// A TCP connection (fsockopen).
    Tcp(std::net::TcpStream),
    /// A child process's output pipe, read through a thread.
    PipeOut(PipeOut),
    /// A child process's input pipe.
    PipeIn(std::process::ChildStdin),
    /// A child process (proc_open); it has no bytes of its own.
    Process(Box<std::process::Child>),
}

/// A pipe read on a thread of its own, so that a read can wait or not
/// (stream_set_blocking) without platform calls: chunks come over a channel,
/// an empty one is the end.
pub struct PipeOut {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    left: Vec<u8>,
    done: bool,
}

impl PipeOut {
    pub fn new(mut reader: impl Read + Send + 'static) -> PipeOut {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(Vec::new());
                        break;
                    }
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        PipeOut { rx, left: Vec::new(), done: false }
    }

    /// Whether a read would not wait: bytes have come, or the end.
    fn ready(&mut self) -> bool {
        if !self.left.is_empty() || self.done {
            return true;
        }
        match self.rx.try_recv() {
            Ok(chunk) => {
                self.done = chunk.is_empty();
                self.left = chunk;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.done = true;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
        }
    }

    /// Up to `n` bytes (none when it would wait, if `nonblocking`), and
    /// whether the pipe has ended.
    fn take(&mut self, n: usize, nonblocking: bool) -> (Vec<u8>, bool) {
        if self.left.is_empty() {
            if self.done {
                return (Vec::new(), true);
            }
            let got = if nonblocking { self.rx.try_recv().map_err(|e| e == std::sync::mpsc::TryRecvError::Disconnected) } else { self.rx.recv().map_err(|_| true) };
            match got {
                Ok(chunk) if chunk.is_empty() => {
                    self.done = true;
                    return (Vec::new(), true);
                }
                Ok(chunk) => self.left = chunk,
                Err(true) => {
                    self.done = true;
                    return (Vec::new(), true);
                }
                Err(false) => return (Vec::new(), false),
            }
        }
        let k = self.left.len().min(n);
        (self.left.drain(..k).collect(), false)
    }
}

/// Opens a URL of a scheme an extension serves (zip://): the function
/// name for warnings, the URL and the fopen mode. The wrapper warns by
/// itself when it cannot open the stream.
pub type WrapperOpen = fn(&mut dyn Ctx, &str, &str, &str) -> Option<Stream>;

static WRAPPERS: std::sync::Mutex<Vec<(&'static str, WrapperOpen)>> = std::sync::Mutex::new(Vec::new());

/// Serve URLs of `scheme` (lowercase, no "://") with `open`.
pub fn register_wrapper(scheme: &'static str, open: WrapperOpen) {
    let mut wrappers = WRAPPERS.lock().unwrap_or_else(|e| e.into_inner());
    if !wrappers.iter().any(|(s, _)| *s == scheme) {
        wrappers.push((scheme, open));
    }
}

/// The schemes extensions registered, in the order they did.
pub fn wrapper_schemes() -> Vec<&'static str> {
    WRAPPERS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(s, _)| *s).collect()
}

/// The opener for `scheme` (lowercase), if an extension registered one.
pub fn wrapper(scheme: &str) -> Option<WrapperOpen> {
    WRAPPERS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(s, _)| *s == scheme).map(|(_, open)| *open)
}

/// What runs with a stream's bytes when it closes.
pub type OnClose = Box<dyn FnOnce(&[u8])>;

pub struct Stream {
    pub target: Target,
    pub path: String,
    pub mode: String,
    /// Read or write position (Memory; Files keep their own).
    pub pos: usize,
    pub eof: bool,
    /// Read ahead of what the script has taken (fgets reads past the line).
    pub buffer: Vec<u8>,
    /// Runs with a Memory target's bytes when the stream closes (or goes
    /// away): a gzip file compressed and written on gzclose.
    pub on_close: Option<OnClose>,
    /// A zlib (gzip file) stream: one way only, no SEEK_END.
    pub zlib: bool,
    /// stream_set_blocking(false): a read does not wait (sockets, pipes).
    pub nonblocking: bool,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.close();
    }
}

impl Stream {
    pub fn new(target: Target, path: &str, mode: &str) -> Stream {
        Stream { target, path: path.to_string(), mode: mode.to_string(), pos: 0, eof: false, buffer: Vec::new(), on_close: None, zlib: false, nonblocking: false }
    }

    /// stream_select's read test: whether a read would not wait (bytes
    /// buffered or arrived, or the end).
    pub fn read_ready(&mut self) -> bool {
        if !self.buffer.is_empty() {
            return true;
        }
        match &mut self.target {
            Target::PipeOut(p) => p.ready(),
            Target::Tcp(sock) => {
                let blocking = !self.nonblocking;
                if blocking && sock.set_nonblocking(true).is_err() {
                    return true;
                }
                let ready = !matches!(sock.peek(&mut [0u8]), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock);
                if blocking {
                    let _ = sock.set_nonblocking(false);
                }
                ready
            }
            _ => true,
        }
    }

    /// Close: flushed, the close hook run, the target let go.
    pub fn close(&mut self) {
        let _ = self.flush();
        let target = std::mem::replace(&mut self.target, Target::Memory(Vec::new()));
        if let (Some(finish), Target::Memory(data)) = (self.on_close.take(), &target) {
            finish(data);
        }
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
            Target::Tcp(sock) => {
                let _ = sock.set_nonblocking(self.nonblocking);
                let mut buf = vec![0; n];
                match sock.read(&mut buf) {
                    Ok(0) => {
                        self.eof = true;
                        Vec::new()
                    }
                    Ok(got) => {
                        buf.truncate(got);
                        buf
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => Vec::new(),
                    Err(_) => {
                        self.eof = true;
                        Vec::new()
                    }
                }
            }
            Target::PipeOut(pipe) => {
                let (bytes, ended) = pipe.take(n, self.nonblocking);
                if ended {
                    self.eof = true;
                }
                bytes
            }
            Target::Stdout | Target::Stderr | Target::PipeIn(_) | Target::Process(_) => Vec::new(),
        }
    }

    /// Whether a short read means the end (files, memory) or only that
    /// nothing more has arrived yet (sockets and pipes).
    fn short_read_is_end(&self) -> bool {
        !matches!(self.target, Target::Tcp(_) | Target::PipeOut(_))
    }

    /// fread: up to `n` bytes.
    pub fn read(&mut self, n: usize) -> Vec<u8> {
        let mut out: Vec<u8> = self.buffer.drain(..self.buffer.len().min(n)).collect();
        if out.len() < n {
            let want = n - out.len();
            let more = self.fill(want);
            // A short read is the end (plain files and memory).
            if more.len() < want && self.short_read_is_end() {
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
            if more.len() < 8192 && self.short_read_is_end() {
                self.eof = true;
            }
            if more.is_empty() {
                if self.buffer.is_empty() {
                    return None;
                }
                // Nothing more yet on a socket or pipe: what there is goes
                // when the stream ended, else the line is not whole.
                if !self.short_read_is_end() && !self.eof {
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
            Target::Tcp(sock) => sock.write_all(bytes).ok().map(|()| bytes.len()),
            Target::PipeIn(pipe) => pipe.write_all(bytes).ok().map(|()| bytes.len()),
            Target::Stdin | Target::PipeOut(_) | Target::Process(_) => None,
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
            Target::Tcp(s) => s.flush().is_ok(),
            Target::PipeIn(p) => p.flush().is_ok(),
            _ => true,
        }
    }

    /// feof: whether a read hit the end (and nothing is buffered).
    pub fn at_end(&self) -> bool {
        self.eof && self.buffer.is_empty()
    }
}
