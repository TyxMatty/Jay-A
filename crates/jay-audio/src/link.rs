//! Jay Link server (docs/ARQUITETURA.md, section 6): Jay-A's end of the named pipe to Jay-C. During
//! a call Jay-C connects, says HELLO, asks for the microphone with `start-mic` and gets the cleaned
//! mic as AUDIO every 20 ms. Framing and the overlapped pipe are adapted from Jay-C's client
//! (plugins/tauri-plugin-system-audio/src/link.rs) and its stand-in Jay-A (link/fake.rs).
//!
//! Frames: opcode (u32 LE) + length (u32 LE) + payload. HELLO and CONTROL carry JSON objects,
//! AUDIO 960 mono f32 LE samples (20 ms at 48 kHz), PING/PONG nothing.
//!
//! One client at a time, local only, and only the current Windows user (DACL). Overlapped I/O: on a
//! synchronous handle, a pending read blocks every write.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{PCWSTR, w};

use crate::{CHUNK_FRAMES, SAMPLE_RATE};

/// The pipe Jay-C looks for: `\\.\pipe\jay-a-link`.
pub const PIPE: &str = "jay-a-link";
pub const PROTOCOL: u64 = 1;

const HELLO: u32 = 0;
const CONTROL: u32 = 1;
const AUDIO: u32 = 2;
const PING: u32 = 3;
const PONG: u32 = 4;
const MAX_MESSAGE: usize = 64 * 1024;
/// A write Jay-C doesn't take in this long means it hung.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Jay-C sends PING every 5 s; nothing heard for this long and it's gone.
const SILENCE: Duration = Duration::from_secs(10);
/// With nothing else to send (mic off), a PING this often keeps Jay-C from giving up on us.
const PING_EVERY: Duration = Duration::from_secs(5);
/// How long a new client has to say HELLO.
const HELLO_WAIT: Duration = Duration::from_secs(5);
/// Chunks waiting for the pipe. Like Jay-C's loopback queues (MAX_QUEUED_CHUNKS): more is a delay
/// the call would keep, so new chunks are dropped instead.
const QUEUED_CHUNKS: usize = 5;
/// Only the current user (the pipe's owner) may open the pipe: other users of the PC don't hear
/// the microphone.
const OWNER_ONLY: PCWSTR = w!("D:P(A;;GA;;;OW)");

/// What Jay-C asks for, as it arrives (on the link's thread).
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// A Jay-C said HELLO. `pid`: its process, whose WebView2 plays the call (the echo reference);
    /// `import`: its microphone settings, for a Jay-A that has none yet.
    Connected { pid: u32, version: String, import: Option<Value> },
    StartMic,
    StopMic,
    /// For the window only: muting in Jay-C pauses the call's track there.
    CallState { in_call: bool, muted: bool },
    /// Open Jay-A's window.
    Show,
    /// The client left, or was dropped; its microphone request ends with it.
    Disconnected,
}

/// Where the microphone's chunks go to reach Jay-C. For the audio thread: never waits.
#[derive(Clone)]
pub struct LinkAudio(SyncSender<[f32; CHUNK_FRAMES]>);

impl LinkAudio {
    /// Dropped when nobody asked for the mic or the queue is full.
    pub fn push(&self, chunk: &[f32; CHUNK_FRAMES]) {
        let _ = self.0.try_send(*chunk);
    }
}

#[derive(Default)]
struct Outbox {
    /// CONTROL messages for the client connected now.
    queue: Vec<Value>,
    /// The last `state`, sent again to each new client.
    state: Option<Value>,
}

pub struct LinkServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    outbox: Arc<Mutex<Outbox>>,
    audio: LinkAudio,
}

impl Drop for LinkServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl LinkServer {
    /// Listens on `\\.\pipe\<name>` (already when this returns) and serves one client at a time.
    /// `version`: Jay-A's, for the HELLO. `on_event` runs on the link's thread; the link waits for it.
    pub fn start(name: &str, version: &str, on_event: impl FnMut(LinkEvent) + Send + 'static) -> Result<LinkServer, String> {
        let first = listen(name)?;
        let stop = Arc::new(AtomicBool::new(false));
        let outbox: Arc<Mutex<Outbox>> = Arc::default();
        let (tx, rx) = sync_channel(QUEUED_CHUNKS);
        let server = Server { name: name.to_string(), version: version.to_string(), stop: stop.clone(), outbox: outbox.clone(), audio: rx };
        let thread = std::thread::Builder::new()
            .name("jay-a-link".into())
            .spawn(move || server.run(first, on_event))
            .map_err(|e| e.to_string())?;
        Ok(LinkServer { stop, thread: Some(thread), outbox, audio: LinkAudio(tx) })
    }

    pub fn audio(&self) -> LinkAudio {
        self.audio.clone()
    }

    /// `{ type: "state", ... }`: sent now if Jay-C is connected, and to each client that connects.
    pub fn set_state(&self, state: Value) {
        let mut outbox = self.outbox.lock().unwrap();
        outbox.state = Some(state.clone());
        outbox.queue.push(state);
    }

    /// Any other CONTROL message, for the client connected now (e.g. `{ type: "error", message }`).
    pub fn send(&self, message: Value) {
        self.outbox.lock().unwrap().queue.push(message);
    }
}

struct Server {
    name: String,
    version: String,
    stop: Arc<AtomicBool>,
    outbox: Arc<Mutex<Outbox>>,
    audio: Receiver<[f32; CHUNK_FRAMES]>,
}

impl Server {
    fn run(self, first: Pipe, mut on_event: impl FnMut(LinkEvent)) {
        crate::audio_thread_priority(); // it writes each chunk as it comes
        let mut pipe = Some(first);
        while !self.stopped() {
            let Some(current) = pipe.as_mut() else {
                // A fresh instance for the next client, created only after the old one is closed:
                // with FILE_FLAG_FIRST_PIPE_INSTANCE, creating it while the old one exists fails.
                match listen(&self.name) {
                    Ok(next) => pipe = Some(next),
                    Err(e) => {
                        log::warn!("jay link: {e}; trying again");
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                continue;
            };
            if !accept(current, &self.stop) {
                continue;
            }
            match self.serve(current, &mut on_event) {
                Ok(()) => log::info!("jay link: Jay-C left"),
                Err(e) => log::warn!("jay link: dropped Jay-C: {e}"),
            }
            pipe = None;
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Serves one client until it leaves (Ok) or breaks the protocol (Err).
    fn serve(&self, pipe: &mut Pipe, on_event: &mut impl FnMut(LinkEvent)) -> Result<(), String> {
        let mut frames = Frames::default();
        let deadline = Instant::now() + HELLO_WAIT;
        let hello = loop {
            if let Some(frame) = frames.next()? {
                break frame;
            }
            if self.stopped() {
                return Ok(());
            }
            let left = deadline.checked_duration_since(Instant::now()).ok_or("no HELLO")?;
            if let Some(bytes) = pipe.read(left.min(Duration::from_millis(50)))? {
                frames.push(bytes);
            }
        };
        let Frame::Hello(hello) = hello else { return Err("the first message wasn't HELLO".into()) };
        if hello["protocol"].as_u64() != Some(PROTOCOL) {
            let error = format!("Jay-A speaks protocol {PROTOCOL}, not {}", hello["protocol"]);
            pipe.write(&json_frame(HELLO, &json!({ "error": error })))?;
            return Err(error);
        }
        pipe.write(&json_frame(
            HELLO,
            &json!({ "app": "jay-a", "version": self.version, "protocol": PROTOCOL, "sampleRate": SAMPLE_RATE, "channels": 1, "frameMs": 20 }),
        ))?;
        {
            let mut outbox = self.outbox.lock().unwrap();
            outbox.queue = outbox.state.iter().cloned().collect();
        }
        let version = hello["version"].as_str().unwrap_or("?").to_string();
        on_event(LinkEvent::Connected { pid: pipe.client_pid(), version, import: hello.get("import").cloned() });
        let result = self.talk(pipe, frames, on_event);
        on_event(LinkEvent::Disconnected);
        result
    }

    fn talk(&self, pipe: &mut Pipe, mut frames: Frames, on_event: &mut impl FnMut(LinkEvent)) -> Result<(), String> {
        let mut mic_on = false;
        let (mut heard, mut wrote) = (Instant::now(), Instant::now());
        let mut audio = Vec::with_capacity(8 + CHUNK_FRAMES * 4);
        while !self.stopped() {
            // The next chunk, or 20 ms: control messages and pings get a look at least that often.
            match self.audio.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) if mic_on => {
                    audio.clear();
                    audio.extend_from_slice(&AUDIO.to_le_bytes());
                    audio.extend_from_slice(&((CHUNK_FRAMES * 4) as u32).to_le_bytes());
                    audio.extend(chunk.iter().flat_map(|s| s.to_le_bytes()));
                    pipe.write(&audio)?;
                    wrote = Instant::now();
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()), // the server is going away
            }
            while let Some(bytes) = pipe.read(Duration::ZERO)? {
                frames.push(bytes);
                heard = Instant::now();
            }
            while let Some(frame) = frames.next()? {
                match frame {
                    Frame::Ping => pipe.write(&frame_bytes(PONG, &[]))?,
                    Frame::Pong => {}
                    Frame::Hello(_) => return Err("a second HELLO".into()),
                    Frame::Control(message) => match message["type"].as_str() {
                        Some("start-mic") => {
                            on_event(LinkEvent::StartMic); // opens the mic: takes a moment
                            while self.audio.try_recv().is_ok() {} // chunks from before the request
                            mic_on = true;
                        }
                        Some("stop-mic") => {
                            mic_on = false;
                            on_event(LinkEvent::StopMic);
                        }
                        Some("call-state") => on_event(LinkEvent::CallState {
                            in_call: message["inCall"].as_bool().unwrap_or(false),
                            muted: message["muted"].as_bool().unwrap_or(false),
                        }),
                        Some("show") => on_event(LinkEvent::Show),
                        // Newer messages (e.g. the soundboard's, stage 8) from a newer Jay-C.
                        _ => log::debug!("jay link: ignored {message}"),
                    },
                }
            }
            let queued = std::mem::take(&mut self.outbox.lock().unwrap().queue);
            for message in queued {
                pipe.write(&json_frame(CONTROL, &message))?;
                wrote = Instant::now();
            }
            if heard.elapsed() > SILENCE {
                return Err("Jay-C stopped answering".into());
            }
            if wrote.elapsed() > PING_EVERY {
                pipe.write(&frame_bytes(PING, &[]))?;
                wrote = Instant::now();
            }
        }
        Ok(())
    }
}

// ---------- Frames ----------

#[derive(Debug, PartialEq)]
enum Frame {
    Hello(Value),
    Control(Value),
    Ping,
    Pong,
}

fn frame_bytes(op: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + payload.len());
    bytes.extend_from_slice(&op.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn json_frame(op: u32, value: &Value) -> Vec<u8> {
    frame_bytes(op, value.to_string().as_bytes())
}

/// Splits the byte stream from Jay-C into frames. Errors (the client is dropped on any): an
/// opcode Jay-C doesn't send (AUDIO included), a message over 64 KB (refused from its header,
/// before it arrives), JSON that isn't an object.
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    fn next(&mut self) -> Result<Option<Frame>, String> {
        if self.buf.len() < 8 {
            return Ok(None);
        }
        let op = u32::from_le_bytes(self.buf[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(self.buf[4..8].try_into().unwrap()) as usize;
        match op {
            HELLO | CONTROL | PING | PONG if len > MAX_MESSAGE => return Err(format!("a message of {len} bytes")),
            HELLO | CONTROL | PING | PONG => {}
            _ => return Err(format!("unexpected opcode {op}")),
        }
        if self.buf.len() < 8 + len {
            return Ok(None);
        }
        let payload: Vec<u8> = self.buf.drain(..8 + len).skip(8).collect();
        Ok(Some(match op {
            PING => Frame::Ping,
            PONG => Frame::Pong,
            _ => {
                let value: Value = serde_json::from_slice(&payload).map_err(|e| format!("invalid JSON: {e}"))?;
                if !value.is_object() {
                    return Err("a message that isn't a JSON object".into());
                }
                if op == HELLO { Frame::Hello(value) } else { Frame::Control(value) }
            }
        }))
    }
}

// ---------- Pipe ----------

/// Creates the pipe's instance for the next client.
fn listen(name: &str) -> Result<Pipe, String> {
    let path = pipe_path(name);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(OWNER_ONLY, SDDL_REVISION_1, &mut descriptor, None) }
        .map_err(|e| format!("can't make the pipe's security: {}", e.message()))?;
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let handle = unsafe {
        let handle = CreateNamedPipeW(
            PCWSTR(path.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            Some(&security),
        );
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        handle
    };
    if handle.is_invalid() {
        return Err(format!("can't create the pipe {name}: {}", windows::core::Error::from_thread().message()));
    }
    Pipe::new(handle)
}

/// Waits for a client; false = stopped or failed (the instance is then replaced).
fn accept(pipe: &mut Pipe, stop: &AtomicBool) -> bool {
    unsafe {
        match ConnectNamedPipe(pipe.handle, Some(&mut *pipe.read_ov)) {
            Ok(()) => return true,
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => return true,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(_) => return false,
        }
        let mut n = 0;
        loop {
            if WaitForSingleObject(pipe.read_ov.hEvent, 50) == WAIT_OBJECT_0 {
                return GetOverlappedResult(pipe.handle, &*pipe.read_ov, &mut n, false).is_ok();
            }
            if stop.load(Ordering::SeqCst) {
                let _ = CancelIoEx(pipe.handle, Some(&*pipe.read_ov));
                let _ = GetOverlappedResult(pipe.handle, &*pipe.read_ov, &mut n, true);
                return false;
            }
        }
    }
}

/// An overlapped pipe handle with one read that may stay pending between calls. The buffer and the
/// OVERLAPPED structs are boxed: Windows writes to them until the operation completes.
struct Pipe {
    handle: HANDLE,
    read_ov: Box<OVERLAPPED>,
    write_ov: Box<OVERLAPPED>,
    buf: Vec<u8>,
    reading: bool,
}

// The handle and its buffers move to the link thread together and are only used there.
unsafe impl Send for Pipe {}

impl Pipe {
    fn new(handle: HANDLE) -> Result<Pipe, String> {
        let mut pipe = Pipe { handle, read_ov: Box::default(), write_ov: Box::default(), buf: vec![0; MAX_MESSAGE], reading: false };
        let event = || unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| e.message());
        pipe.read_ov.hEvent = event()?;
        pipe.write_ov.hEvent = event()?;
        Ok(pipe)
    }

    /// Waits up to `timeout` for bytes. `None` = nothing yet (the read stays pending).
    fn read(&mut self, timeout: Duration) -> Result<Option<&[u8]>, String> {
        unsafe {
            if !self.reading {
                match ReadFile(self.handle, Some(&mut self.buf), None, Some(&mut *self.read_ov)) {
                    Ok(()) => {}
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                    Err(e) => return Err(e.message()),
                }
                self.reading = true;
            }
            if WaitForSingleObject(self.read_ov.hEvent, millis(timeout)) != WAIT_OBJECT_0 {
                return Ok(None);
            }
            self.reading = false;
            let mut n = 0;
            GetOverlappedResult(self.handle, &*self.read_ov, &mut n, false).map_err(|e| e.message())?;
            Ok(Some(&self.buf[..n as usize]))
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            match WriteFile(self.handle, Some(bytes), None, Some(&mut *self.write_ov)) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                Err(e) => return Err(e.message()),
            }
            let mut n = 0;
            if WaitForSingleObject(self.write_ov.hEvent, millis(WRITE_TIMEOUT)) != WAIT_OBJECT_0 {
                // Wait for the cancel to land before `bytes` can be freed.
                let _ = CancelIoEx(self.handle, Some(&*self.write_ov));
                let _ = GetOverlappedResult(self.handle, &*self.write_ov, &mut n, true);
                return Err("Jay-C isn't reading".into());
            }
            GetOverlappedResult(self.handle, &*self.write_ov, &mut n, false).map_err(|e| e.message())?;
            if n as usize != bytes.len() {
                return Err(format!("wrote {n} of {} bytes", bytes.len()));
            }
            Ok(())
        }
    }

    /// The connected client's process id (0 if Windows won't say).
    fn client_pid(&self) -> u32 {
        let mut pid = 0;
        let _ = unsafe { GetNamedPipeClientProcessId(self.handle, &mut pid) };
        pid
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            if self.reading {
                // Windows may still write into `buf`: cancel and wait before freeing it.
                let _ = CancelIoEx(self.handle, Some(&*self.read_ov));
                let mut n = 0;
                let _ = GetOverlappedResult(self.handle, &*self.read_ov, &mut n, true);
            }
            for handle in [self.handle, self.read_ov.hEvent, self.write_ov.hEvent] {
                if !handle.is_invalid() {
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u32::MAX as u128 - 1) as u32
}

fn pipe_path(name: &str) -> Vec<u16> {
    format!(r"\\.\pipe\{name}").encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING};

    use super::*;

    /// Jay-C's end, enough to test the server: sends frames and reads any frame back.
    struct Client {
        pipe: Pipe,
        buf: Vec<u8>,
    }

    impl Client {
        fn connect(name: &str) -> Client {
            let path = pipe_path(name);
            let handle = unsafe {
                CreateFileW(PCWSTR(path.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_MODE(0), None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
            }
            .expect("the server is listening");
            Client { pipe: Pipe::new(handle).unwrap(), buf: Vec::new() }
        }

        fn hello(name: &str, protocol: u64) -> (Client, Value) {
            let mut client = Client::connect(name);
            client.send(&json_frame(HELLO, &json!({ "app": "jay-c", "version": "test", "protocol": protocol, "import": { "micDevice": "x" } })));
            let (op, reply) = client.next(Duration::from_secs(2)).expect("a HELLO back");
            assert_eq!(op, HELLO);
            (client, serde_json::from_slice(&reply).unwrap())
        }

        fn send(&mut self, bytes: &[u8]) {
            self.pipe.write(bytes).unwrap();
        }

        fn control(&mut self, message: Value) {
            self.send(&json_frame(CONTROL, &message));
        }

        /// The next frame (opcode, payload), or None after `timeout` or when the server closed.
        fn next(&mut self, timeout: Duration) -> Option<(u32, Vec<u8>)> {
            let end = Instant::now() + timeout;
            loop {
                if self.buf.len() >= 8 {
                    let len = u32::from_le_bytes(self.buf[4..8].try_into().unwrap()) as usize;
                    if self.buf.len() >= 8 + len {
                        let op = u32::from_le_bytes(self.buf[0..4].try_into().unwrap());
                        return Some((op, self.buf.drain(..8 + len).skip(8).collect()));
                    }
                }
                let left = end.checked_duration_since(Instant::now())?;
                match self.pipe.read(left) {
                    Ok(Some(bytes)) => self.buf.extend_from_slice(bytes),
                    Ok(None) | Err(_) => return None,
                }
            }
        }

        /// Frames until one with `op` (skipping AUDIO and pings meanwhile).
        fn wait_for(&mut self, op: u32) -> Vec<u8> {
            loop {
                let (got, payload) = self.next(Duration::from_secs(2)).unwrap_or_else(|| panic!("no frame {op}"));
                if got == op {
                    return payload;
                }
            }
        }
    }

    fn test_pipe(name: &str) -> String {
        format!("jay-a-link-test-{}-{name}", std::process::id())
    }

    fn server(name: &str) -> (LinkServer, mpsc::Receiver<LinkEvent>) {
        let (tx, rx) = mpsc::channel();
        let server = LinkServer::start(name, "9.9.9", move |event| drop(tx.send(event))).unwrap();
        (server, rx)
    }

    fn event(events: &mpsc::Receiver<LinkEvent>) -> LinkEvent {
        events.recv_timeout(Duration::from_secs(2)).expect("an event")
    }

    #[test]
    fn hello_mic_and_pings() {
        let name = test_pipe("mic");
        let (server, events) = server(&name);
        let (mut client, hello) = Client::hello(&name, PROTOCOL);
        assert_eq!(hello, json!({ "app": "jay-a", "version": "9.9.9", "protocol": 1, "sampleRate": 48000, "channels": 1, "frameMs": 20 }));
        let LinkEvent::Connected { pid, version, import } = event(&events) else { panic!("not Connected") };
        assert_eq!((pid, version.as_str(), import), (std::process::id(), "test", Some(json!({ "micDevice": "x" }))));

        // The mic's chunks, every 20 ms. Before start-mic, they don't go to Jay-C.
        let audio = server.audio();
        let feeding = Arc::new(AtomicBool::new(true));
        let feeder = std::thread::spawn({
            let feeding = feeding.clone();
            move || {
                while feeding.load(Ordering::SeqCst) {
                    audio.push(&[0.25; CHUNK_FRAMES]);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(client.next(Duration::from_millis(100)).is_none(), "nothing before start-mic");

        client.control(json!({ "type": "start-mic" }));
        assert_eq!(event(&events), LinkEvent::StartMic);
        let chunk = client.wait_for(AUDIO);
        assert_eq!(chunk.len(), 3840);
        assert!(chunk.as_chunks::<4>().0.iter().all(|&b| f32::from_le_bytes(b) == 0.25));

        client.control(json!({ "type": "call-state", "inCall": true, "muted": true }));
        assert_eq!(event(&events), LinkEvent::CallState { in_call: true, muted: true });
        client.control(json!({ "type": "show" }));
        assert_eq!(event(&events), LinkEvent::Show);
        client.control(json!({ "type": "soundboard-list" })); // from a newer Jay-C: ignored
        client.send(&frame_bytes(PING, &[]));
        assert!(client.wait_for(PONG).is_empty());

        server.set_state(json!({ "type": "state", "device": "Mic", "deviceId": "id", "preset": null }));
        let state: Value = serde_json::from_slice(&client.wait_for(CONTROL)).unwrap();
        assert_eq!(state["deviceId"], "id");

        client.control(json!({ "type": "stop-mic" }));
        assert_eq!(event(&events), LinkEvent::StopMic);
        drop(client);
        assert_eq!(event(&events), LinkEvent::Disconnected);
        feeding.store(false, Ordering::SeqCst);
        feeder.join().unwrap();
    }

    #[test]
    fn serves_the_next_client_after_one_leaves() {
        let name = test_pipe("again");
        let (server, events) = server(&name);
        server.set_state(json!({ "type": "state", "deviceId": "id" }));
        for _ in 0..3 {
            let (mut client, hello) = Client::hello(&name, PROTOCOL);
            assert_eq!(hello["protocol"], 1);
            assert!(matches!(event(&events), LinkEvent::Connected { .. }));
            // Each new client gets the last state.
            let state: Value = serde_json::from_slice(&client.wait_for(CONTROL)).unwrap();
            assert_eq!(state["deviceId"], "id");
            drop(client);
            assert_eq!(event(&events), LinkEvent::Disconnected);
            std::thread::sleep(Duration::from_millis(100)); // the server listens again
        }
    }

    #[test]
    fn another_protocol_gets_an_error() {
        let name = test_pipe("protocol");
        let (_server, events) = server(&name);
        let (mut client, hello) = Client::hello(&name, 2);
        assert!(hello["error"].as_str().unwrap().contains("protocol 1"), "{hello}");
        assert!(client.next(Duration::from_secs(1)).is_none(), "and closes");
        assert!(events.recv_timeout(Duration::from_millis(200)).is_err(), "no Connected");
    }

    #[test]
    fn breaking_the_protocol_drops_the_client() {
        let name = test_pipe("garbage");
        let (_server, events) = server(&name);
        for bad in [frame_bytes(AUDIO, &[0; 3840]), frame_bytes(9, &[]), json_frame(CONTROL, &json!([1])), frame_bytes(CONTROL, b"{nope")] {
            let (mut client, _) = Client::hello(&name, PROTOCOL);
            assert!(matches!(event(&events), LinkEvent::Connected { .. }));
            client.send(&bad);
            assert_eq!(event(&events), LinkEvent::Disconnected);
            assert!(client.next(Duration::from_secs(1)).is_none(), "closed");
            std::thread::sleep(Duration::from_millis(100));
        }
        // A huge length is refused from the header alone.
        let mut frames = Frames::default();
        frames.push(&CONTROL.to_le_bytes());
        frames.push(&(MAX_MESSAGE as u32 + 1).to_le_bytes());
        assert_eq!(frames.next().unwrap_err(), "a message of 65537 bytes");
    }

    #[test]
    fn the_pipe_is_only_for_this_user() {
        use windows::Win32::Security::Authorization::{ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT};
        use windows::Win32::Security::DACL_SECURITY_INFORMATION;
        // GENERIC_READ (the test client's) includes READ_CONTROL, enough to read the DACL.
        let name = test_pipe("dacl");
        let (_server, _) = server(&name);
        let client = Client::connect(&name);
        let sddl = unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            assert!(GetSecurityInfo(client.pipe.handle, SE_KERNEL_OBJECT, DACL_SECURITY_INFORMATION, None, None, None, None, Some(&mut sd)).is_ok());
            let mut text = windows::core::PWSTR::null();
            ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, DACL_SECURITY_INFORMATION, &mut text, None).unwrap();
            let sddl = text.to_string().unwrap();
            let _ = LocalFree(Some(HLOCAL(sd.0)));
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sddl
        };
        // Protected (nothing inherited), one ACE: full access for the owner, nobody else.
        assert_eq!(sddl, "D:P(A;;FA;;;OW)");
    }

    #[test]
    fn a_second_server_on_the_same_name_fails() {
        let name = test_pipe("twice");
        let (_first, _) = server(&name);
        assert!(LinkServer::start(&name, "x", |_| {}).is_err());
    }
}
