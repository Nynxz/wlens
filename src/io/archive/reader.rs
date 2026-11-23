//! Read-only libarchive interface. Register only wlens's supported formats,
//! before opening any input. A filter warning can mean an external program
//! would be used, so fail closed if a native filter is unavailable.
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::ptr::{self, NonNull};

use anyhow::{Result, anyhow, ensure};

#[link(name = "archive")]
unsafe extern "C" {
    fn archive_read_new() -> *mut c_void;
    fn archive_read_free(a: *mut c_void) -> c_int;
    fn archive_read_support_format_zip(a: *mut c_void) -> c_int;
    fn archive_read_support_format_tar(a: *mut c_void) -> c_int;
    fn archive_read_support_format_rar(a: *mut c_void) -> c_int;
    fn archive_read_support_format_rar5(a: *mut c_void) -> c_int;
    fn archive_read_support_format_7zip(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_gzip(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_bzip2(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_xz(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_zstd(a: *mut c_void) -> c_int;
    fn archive_read_open_fd(a: *mut c_void, fd: c_int, block_size: usize) -> c_int;
    fn archive_read_next_header(a: *mut c_void, entry: *mut *mut c_void) -> c_int;
    fn archive_read_data(a: *mut c_void, buf: *mut c_void, size: usize) -> isize;
    fn archive_entry_pathname_utf8(entry: *mut c_void) -> *const c_char;
    fn archive_entry_pathname(entry: *mut c_void) -> *const c_char;
    fn archive_entry_filetype(entry: *mut c_void) -> libc::mode_t;
    fn archive_entry_hardlink(entry: *mut c_void) -> *const c_char;
    fn archive_entry_size(entry: *mut c_void) -> i64;
    fn archive_error_string(a: *mut c_void) -> *const c_char;
}

// libarchive uses the calling thread's locale when it converts entry names.
struct Utf8Locale {
    current: libc::locale_t,
    previous: libc::locale_t,
}

impl Utf8Locale {
    fn enter() -> Result<Self> {
        unsafe {
            let current =
                libc::newlocale(libc::LC_CTYPE_MASK, c"C.UTF-8".as_ptr(), ptr::null_mut());
            ensure!(!current.is_null(), "UTF-8 locale unavailable");
            let previous = libc::uselocale(current);
            if previous.is_null() {
                libc::freelocale(current);
                return Err(io::Error::last_os_error().into());
            }
            Ok(Self { current, previous })
        }
    }
}

impl Drop for Utf8Locale {
    fn drop(&mut self) {
        // Reader is thread-local (its native pointers are not Send).
        unsafe {
            libc::uselocale(self.previous);
            libc::freelocale(self.current);
        }
    }
}

pub(super) struct Reader {
    handle: NonNull<c_void>,
    file: File,
    _locale: Utf8Locale,
}

impl Reader {
    pub fn new(file: File) -> Result<Self> {
        // SAFETY: this reader owns the native handle and keeps its fd open until
        // after archive_read_free. All setup completes before the first read.
        let locale = Utf8Locale::enter()?;
        unsafe {
            let handle =
                NonNull::new(archive_read_new()).ok_or_else(|| anyhow!("create archive reader"))?;
            let reader = Self {
                handle,
                file,
                _locale: locale,
            };
            let a = handle.as_ptr();
            for register in [
                archive_read_support_format_zip,
                archive_read_support_format_tar,
                archive_read_support_format_rar,
                archive_read_support_format_rar5,
                archive_read_support_format_7zip,
                archive_read_support_filter_gzip,
                archive_read_support_filter_bzip2,
                archive_read_support_filter_xz,
                archive_read_support_filter_zstd,
            ] {
                ensure!(register(a) == 0, "{}", reader.error());
            }
            ensure!(
                archive_read_open_fd(a, reader.file.as_raw_fd(), 64 * 1024) == 0,
                "{}",
                reader.error()
            );
            Ok(reader)
        }
    }

    fn error(&self) -> String {
        // SAFETY: libarchive owns this optional string until the next call.
        unsafe {
            let p = archive_error_string(self.handle.as_ptr());
            if p.is_null() {
                "archive read error".into()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    pub fn header(&mut self) -> Result<Option<(String, bool, u64)>> {
        // SAFETY: inspect only a successful, non-null header. Copy the name
        // before advancing; libarchive owns the entry and all accessor results.
        unsafe {
            let mut entry = ptr::null_mut();
            let rc = archive_read_next_header(self.handle.as_ptr(), &mut entry);
            if rc == 1 {
                return Ok(None);
            } // ARCHIVE_EOF
            ensure!(rc == 0 && !entry.is_null(), "{}", self.error());
            let mut p = archive_entry_pathname_utf8(entry);
            if p.is_null() {
                p = archive_entry_pathname(entry);
            }
            ensure!(!p.is_null(), "missing archive entry name");
            let name = CStr::from_ptr(p).to_bytes();
            ensure!(name.len() <= 4096, "archive entry name too long");
            let regular = archive_entry_filetype(entry) == libc::S_IFREG
                && archive_entry_hardlink(entry).is_null();
            Ok(Some((
                std::str::from_utf8(name)?.to_owned(),
                regular,
                archive_entry_size(entry).max(0) as u64,
            )))
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: the writable slice remains valid for this synchronous call.
        let n =
            unsafe { archive_read_data(self.handle.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            Err(io::Error::other(self.error()))
        } else {
            Ok(n as usize)
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            archive_read_free(self.handle.as_ptr());
        }
    }
}
