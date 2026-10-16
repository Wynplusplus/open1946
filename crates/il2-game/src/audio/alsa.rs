//! Minimal ALSA playback through `libasound.so.2`, loaded at run time (no build-time headers).

use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type WriteI = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;

const PLAYBACK: c_int = 0;
const FORMAT_S16_LE: c_int = 2;
const ACCESS_RW_INTERLEAVED: c_int = 3;

pub struct Pcm {
    lib: Library,
    pcm: *mut c_void,
    pub rate: u32,
}

// The handle is used from one thread at a time (the audio thread).
unsafe impl Send for Pcm {}

impl Pcm {
    /// Opens the default device for 16-bit stereo at `rate` Hz with about `latency_ms` of buffering.
    pub fn open(rate: u32, latency_ms: u32) -> Result<Self, String> {
        // SAFETY: loading the system ALSA library and calling its documented C API.
        unsafe {
            let lib = Library::new("libasound.so.2").map_err(|e| e.to_string())?;
            let open: Symbol<Open> = lib.get(b"snd_pcm_open\0").map_err(|e| e.to_string())?;
            let set: Symbol<SetParams> = lib.get(b"snd_pcm_set_params\0").map_err(|e| e.to_string())?;
            let mut pcm = std::ptr::null_mut();
            let name = CString::new("default").unwrap();
            let r = open(&mut pcm, name.as_ptr(), PLAYBACK, 0);
            if r < 0 {
                return Err(format!("snd_pcm_open: {r}"));
            }
            let r = set(pcm, FORMAT_S16_LE, ACCESS_RW_INTERLEAVED, 2, rate, 1, latency_ms * 1000);
            if r < 0 {
                return Err(format!("snd_pcm_set_params: {r}"));
            }
            Ok(Pcm { lib, pcm, rate })
        }
    }

    /// Writes interleaved stereo frames, blocking until the device takes them.
    pub fn write(&mut self, frames: &[i16]) {
        // SAFETY: `pcm` is an open handle; the buffer outlives the call.
        unsafe {
            let (Ok(w), Ok(rec)) = (self.lib.get::<WriteI>(b"snd_pcm_writei\0"), self.lib.get::<Recover>(b"snd_pcm_recover\0")) else { return };
            let mut off = 0;
            while off < frames.len() / 2 {
                let n = w(self.pcm, frames[off * 2..].as_ptr() as *const c_void, (frames.len() / 2 - off) as c_ulong);
                if n < 0 {
                    if rec(self.pcm, n as c_int, 1) < 0 {
                        return;
                    }
                } else {
                    off += n as usize;
                }
            }
        }
    }
}
