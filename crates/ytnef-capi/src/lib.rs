//! Drop-in C ABI for `libytnef.so.0`.
//!
//! Every function and global the C library exports, with the original
//! headers (`include/`) as the contract. Parsing goes through `ytnef-core`
//! (bounded by [`Limits`]); the result is copied into the C structs with
//! `malloc`-family buffers, so C callers keep calling `free()` and
//! `TNEFFree()` as before. No panic crosses the boundary: each entry point
//! runs under `catch_unwind` and reports a panic as an error return.
//!
//! `TNEFParse` drains the caller's `ReadProc` into memory (at most
//! `Limits::max_input`), validates the whole stream with
//! `ytnef_core::parse_with`, then dispatches every attribute through the
//! exported `TNEFList`, as the C library does. Text goes to C `stdout` via
//! `printf`, so it interleaves with the caller's own output.

#![allow(non_snake_case, non_upper_case_globals, clippy::missing_safety_doc)]

mod types;

pub use types::*;

use libc::{c_char, c_int, c_long, c_uint, c_void};
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{self, null_mut};
use ytnef_core::attr::id as att;
use ytnef_core::mapi::{self, pt, NameKind};
use ytnef_core::rtf;
use ytnef_core::{Error, HandlerError, Limits, Options, RtfError};

pub const YTNEF_CANNOT_INIT_DATA: c_int = -1;
pub const YTNEF_NOT_TNEF_STREAM: c_int = -2;
pub const YTNEF_ERROR_READING_DATA: c_int = -3;
pub const YTNEF_NO_KEY: c_int = -4;
pub const YTNEF_BAD_CHECKSUM: c_int = -5;
pub const YTNEF_ERROR_IN_HANDLER: c_int = -6;
pub const YTNEF_UNKNOWN_PROPERTY: c_int = -7;
pub const YTNEF_INCORRECT_SETUP: c_int = -8;

/// `MAPI_UNDEFINED` from `mapi.h`.
pub const MAPI_UNDEFINED: *mut variableLength = usize::MAX as *mut variableLength;

const MIB: usize = 1024 * 1024;
const PT_UNSPECIFIED: u16 = 0x0000;
const PT_NULL: u16 = 0x0001;
/// Property IDs C copies into an empty message subject.
const SUBJECT_IDS: [u16; 5] = [0x0037, 0x0038, 0x0049, 0x0070, 0x0E1D];

// ---------------------------------------------------------------------------
// helpers

/// Run `f`; a panic becomes `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// Write raw bytes to C `stdout`.
fn out(b: &[u8]) {
    for (i, run) in b.split(|&c| c == 0).enumerate() {
        // SAFETY: the format consumes exactly one int and one pointer to
        // `run.len()` readable bytes.
        unsafe {
            if i > 0 {
                libc::putchar(0);
            }
            for part in run.chunks(c_int::MAX as usize) {
                libc::printf(c"%.*s".as_ptr(), part.len() as c_int, part.as_ptr());
            }
        }
    }
}

macro_rules! outf {
    ($($a:tt)*) => { out(format!($($a)*).as_bytes()) };
}

/// What C `%.*s` prints of `b`: up to the first NUL.
fn upto_nul(b: &[u8]) -> &[u8] {
    match b.iter().position(|&c| c == 0) {
        Some(n) => &b[..n],
        None => b,
    }
}

unsafe fn bytes<'a>(p: *const c_char, n: c_int) -> &'a [u8] {
    if p.is_null() || n <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(p.cast(), n as usize)
    }
}

unsafe fn vl_bytes<'a>(v: *const variableLength) -> &'a [u8] {
    if v.is_null() {
        &[]
    } else {
        bytes((*v).data.cast(), (*v).size)
    }
}

unsafe fn vl_free(v: &mut variableLength) {
    if !v.data.is_null() {
        libc::free(v.data.cast());
    }
    v.data = null_mut();
    v.size = 0;
}

/// `calloc(len + extra)` holding a copy of `src`.
unsafe fn dup(src: &[u8], extra: usize) -> *mut BYTE {
    let p = libc::calloc(src.len().saturating_add(extra).max(1), 1).cast::<BYTE>();
    if !p.is_null() {
        ptr::copy_nonoverlapping(src.as_ptr(), p, src.len());
    }
    p
}

fn oom(line: u32) -> c_int {
    outf!("Out of Memory at {} : {}\n", file!(), line);
    -1
}

/// Replace `v` with a copy of `src` plus `extra` zero bytes.
unsafe fn vl_set(v: &mut variableLength, src: &[u8], extra: usize) -> c_int {
    vl_free(v);
    let p = dup(src, extra);
    if p.is_null() {
        return oom(line!());
    }
    v.data = p;
    v.size = src.len() as c_int;
    0
}

/// C `PREALLOCCHECK(sz, max)`.
fn prealloc(sz: c_int, max: usize) -> bool {
    let u = sz as c_uint;
    if u == 0 || u as usize > max {
        outf!(
            "ERROR: invalid alloc size {} at {} : {}, suspected corruption (exceeded {} bytes)\n",
            u,
            file!(),
            line!(),
            max
        );
        return false;
    }
    true
}

fn corrupted(line: u32) -> c_int {
    outf!("Corrupted file detected at {} : {}\n", file!(), line);
    -1
}

fn le16(b: &[u8]) -> u16 {
    let mut a = [0; 2];
    let n = b.len().min(2);
    a[..n].copy_from_slice(&b[..n]);
    u16::from_le_bytes(a)
}

fn le32(b: &[u8]) -> u32 {
    let mut a = [0; 4];
    let n = b.len().min(4);
    a[..n].copy_from_slice(&b[..n]);
    u32::from_le_bytes(a)
}

fn le64(b: &[u8]) -> u64 {
    let mut a = [0; 8];
    let n = b.len().min(8);
    a[..n].copy_from_slice(&b[..n]);
    u64::from_le_bytes(a)
}

/// Copy up to `n` bytes of `src` over the start of a C char array.
fn copy_into(dst: &mut [c_char], src: &[u8], n: usize) {
    for (d, &s) in dst.iter_mut().zip(src).take(n) {
        *d = s as c_char;
    }
}

/// Write `s` into a C char array, truncated and NUL-terminated like `snprintf`.
fn set_cstr(dst: &mut [c_char], s: &str) {
    let n = s.len().min(dst.len().saturating_sub(1));
    copy_into(dst, s.as_bytes(), n);
    if let Some(d) = dst.get_mut(n) {
        *d = 0;
    }
}

unsafe fn last_attachment(t: *mut TNEFStruct) -> *mut Attachment {
    let mut p = &raw mut (*t).starting_attach;
    while !(*p).next.is_null() {
        p = (*p).next;
    }
    p
}

unsafe fn handler_entry(id: c_int) -> Option<TNEFHandler> {
    let list = &raw const TNEFList;
    usize::try_from(id)
        .ok()
        .and_then(|i| (*list).get(i).copied())
}

unsafe fn handler_name(id: c_int) -> Vec<u8> {
    handler_entry(id)
        .map(|h| upto_nul(&h.name.map(|c| c as u8)).to_vec())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// exported globals

const fn cname<const N: usize>(s: &str) -> [c_char; N] {
    let b = s.as_bytes();
    let mut a = [0 as c_char; N];
    let mut i = 0;
    while i < b.len() && i + 1 < N {
        a[i] = b[i] as c_char;
        i += 1;
    }
    a
}

const fn entry(id: DWORD, name: &str, handler: Handler) -> TNEFHandler {
    TNEFHandler {
        id,
        name: cname(name),
        handler: Some(handler),
    }
}

/// The attribute dispatch table. Mutable, as in C: `TNEFParse` calls
/// whatever handler a caller installs here.
#[no_mangle]
pub static mut TNEFList: [TNEFHandler; 33] = [
    entry(att::NULL, "Null", TNEFDefaultHandler),
    entry(att::FROM, "From", TNEFFromHandler),
    entry(att::SUBJECT, "Subject", TNEFSubjectHandler),
    entry(att::DATE_SENT, "Date Sent", TNEFDateHandler),
    entry(att::DATE_RECD, "Date Received", TNEFDateHandler),
    entry(att::MESSAGE_STATUS, "Message Status", TNEFDefaultHandler),
    entry(att::MESSAGE_CLASS, "Message Class", TNEFMessageClass),
    entry(att::MESSAGE_ID, "Message ID", TNEFMessageID),
    entry(att::PARENT_ID, "Parent ID", TNEFParentID),
    entry(att::CONVERSATION_ID, "Conversation ID", TNEFDefaultHandler),
    entry(att::BODY, "Body", TNEFBody),
    entry(att::PRIORITY, "Priority", TNEFPriority),
    entry(att::ATTACH_DATA, "Attach Data", TNEFAttachmentSave),
    entry(att::ATTACH_TITLE, "Attach Title", TNEFAttachmentFilename),
    entry(att::ATTACH_META_FILE, "Attach Meta-File", TNEFIcon),
    entry(
        att::ATTACH_CREATE_DATE,
        "Attachment Create Date",
        TNEFDateHandler,
    ),
    entry(
        att::ATTACH_MODIFY_DATE,
        "Attachment Modify Date",
        TNEFDateHandler,
    ),
    entry(att::DATE_MODIFIED, "Date Modified", TNEFDateHandler),
    entry(
        att::ATTACH_TRANSPORT_FILENAME,
        "Attachment Transport name",
        TNEFDefaultHandler,
    ),
    entry(
        att::ATTACH_RENDDATA,
        "Attachment Display info",
        TNEFRendData,
    ),
    entry(att::MAPI_PROPS, "MAPI Properties", TNEFMapiProperties),
    entry(att::RECIP_TABLE, "Recip Table", TNEFRecipTable),
    entry(att::ATTACHMENT, "Attachment", TNEFAttachmentMAPI),
    entry(att::TNEF_VERSION, "TNEF Version", TNEFVersion),
    entry(att::OEM_CODEPAGE, "OEM CodePage", TNEFCodePage),
    entry(
        att::ORIGINAL_MESSAGE_CLASS,
        "Original Message Class",
        TNEFOriginalMsgClass,
    ),
    entry(att::OWNER, "Owner", TNEFDefaultHandler),
    entry(att::SENT_FOR, "Sent For", TNEFSentFor),
    entry(att::DELEGATE, "Delegate", TNEFDefaultHandler),
    entry(att::DATE_START, "Date Start", TNEFDateHandler),
    entry(att::DATE_END, "Date End", TNEFDateHandler),
    entry(att::AID_OWNER, "Aid Owner", TNEFDefaultHandler),
    entry(att::REQUEST_RES, "Request Response", TNEFDefaultHandler),
];

const NTAGS: usize = ytnef_core::tags::TAGS.len();

const fn mplist() -> [MAPIPropertyTagList; NTAGS] {
    let mut a = [const {
        MAPIPropertyTagList {
            id: 0,
            name: [0; 128],
        }
    }; NTAGS];
    let mut i = 0;
    while i < NTAGS {
        let (id, name) = ytnef_core::tags::TAGS[i];
        a[i] = MAPIPropertyTagList {
            id: id as c_uint,
            name: cname(name),
        };
        i += 1;
    }
    a
}

/// `PR_*` names, as in `mapitags.h`.
#[no_mangle]
pub static MPList: [MAPIPropertyTagList; NTAGS] = mplist();

// Unused by the library; exported because the C library exports them.
#[no_mangle]
pub static mut TNEFFileContents: *mut BYTE = null_mut();
#[no_mangle]
pub static mut TNEFFileContentsSize: DWORD = 0;
#[no_mangle]
pub static mut TNEFFileIcon: *mut BYTE = null_mut();
#[no_mangle]
pub static mut TNEFFileIconSize: DWORD = 0;

// ---------------------------------------------------------------------------
// byte order and text

#[no_mangle]
pub unsafe extern "C" fn SwapWord(p: *mut BYTE, size: c_int) -> WORD {
    guard(0, || unsafe { le16(bytes(p.cast(), size.min(2))) })
}

#[no_mangle]
pub unsafe extern "C" fn SwapDWord(p: *mut BYTE, size: c_int) -> DWORD {
    guard(0, || unsafe { le32(bytes(p.cast(), size.min(4))) })
}

#[no_mangle]
pub unsafe extern "C" fn SwapDDWord(p: *mut BYTE, size: c_int) -> DDWORD {
    guard(0, || unsafe { le64(bytes(p.cast(), size.min(8))) })
}

/// UTF-16LE to UTF-8 as C `to_utf8`: `calloc(3 * len / 2 + 1)`, unpaired
/// surrogates become U+FFFD, NUL-terminated.
unsafe fn utf8_from_utf16(b: &[u8]) -> *mut BYTE {
    let len = b.len();
    let p = libc::calloc(3 * len / 2 + 1, 1).cast::<BYTE>();
    if p.is_null() {
        return p;
    }
    let mut s = Vec::with_capacity(3 * len / 2);
    let mut i = 0;
    while i + 1 < len {
        let mut c = u32::from(le16(&b[i..]));
        if (0xd800..=0xdbff).contains(&c) && i + 3 < len {
            let lo = u32::from(le16(&b[i + 2..]));
            if (0xdc00..=0xdfff).contains(&lo) {
                c = 0x10000 + ((c - 0xd800) << 10) + (lo - 0xdc00);
                s.extend_from_slice(&[
                    0xf0 | ((c >> 18) & 0x07) as u8,
                    0x80 | ((c >> 12) & 0x3f) as u8,
                    0x80 | ((c >> 6) & 0x3f) as u8,
                    0x80 | (c & 0x3f) as u8,
                ]);
                i += 4;
                continue;
            }
        }
        if (0xd800..=0xdfff).contains(&c) {
            c = 0xfffd;
        }
        if c <= 0x7f {
            s.push(c as u8);
        } else if c <= 0x7ff {
            s.extend_from_slice(&[0xc0 | ((c & 0x07c0) >> 6) as u8, 0x80 | (c & 0x3f) as u8]);
        } else {
            s.extend_from_slice(&[
                0xe0 | ((c & 0xf000) >> 12) as u8,
                0x80 | ((c & 0x0fc0) >> 6) as u8,
                0x80 | (c & 0x3f) as u8,
            ]);
        }
        i += 2;
    }
    ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
    p
}

#[no_mangle]
pub unsafe extern "C" fn to_utf8(len: usize, buf: *mut c_char) -> *mut c_char {
    guard(null_mut(), || unsafe {
        let b = if buf.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(buf.cast::<u8>(), len)
        };
        utf8_from_utf16(b).cast()
    })
}

// ---------------------------------------------------------------------------
// attribute handlers (STD_ARGLIST)

macro_rules! handler {
    ($(#[$m:meta])* $name:ident($t:ident, $id:ident, $d:ident, $size:ident) $body:block) => {
        $(#[$m])*
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            tnef: *mut TNEFStruct,
            id: c_int,
            data: *mut c_char,
            size: c_int,
        ) -> c_int {
            guard(-1, || unsafe {
                if tnef.is_null() || size < 0 || (data.is_null() && size > 0) {
                    return -1;
                }
                #[allow(unused_variables)]
                let ($t, $id, $d, $size) = (tnef, id, bytes(data, size), size);
                $body
            })
        }
    };
}

handler!(TNEFDefaultHandler(t, id, d, size) {
    if (*t).Debug >= 1 {
        let mut s = handler_name(id);
        s.extend_from_slice(format!(": [{size}] ").as_bytes());
        s.extend_from_slice(upto_nul(d));
        s.push(b'\n');
        out(&s);
    }
    0
});

handler!(TNEFCodePage(t, id, d, size) { vl_set(&mut (*t).CodePage, d, 0) });

handler!(TNEFParentID(t, id, d, size) {
    copy_into(&mut (*t).parentID, d, 50);
    0
});

handler!(TNEFMessageID(t, id, d, size) {
    copy_into(&mut (*t).messageID, d, 50);
    0
});

handler!(TNEFBody(t, id, d, size) {
    vl_free(&mut (*t).body);
    if !prealloc(size, 100_000) {
        return -1;
    }
    vl_set(&mut (*t).body, d, 1)
});

handler!(TNEFOriginalMsgClass(t, id, d, size) {
    vl_free(&mut (*t).OriginalMessageClass);
    if !prealloc(size, 100) {
        return -1;
    }
    vl_set(&mut (*t).OriginalMessageClass, d, 1)
});

handler!(TNEFMessageClass(t, id, d, size) {
    copy_into(&mut (*t).messageClass, d, 49);
    0
});

handler!(TNEFFromHandler(t, id, d, size) {
    vl_free(&mut (*t).from);
    if !prealloc(size, 100) {
        return -1;
    }
    vl_set(&mut (*t).from, d, 1)
});

handler!(TNEFSubjectHandler(t, id, d, size) {
    vl_free(&mut (*t).subject);
    if !prealloc(size, 1000) {
        return -1;
    }
    vl_set(&mut (*t).subject, d, 1)
});

handler!(TNEFRendData(t, id, d, size) {
    let last = last_attachment(t);
    let p = libc::calloc(1, size_of::<Attachment>()).cast::<Attachment>();
    if p.is_null() {
        return oom(line!());
    }
    (*last).next = p;
    // C copies the raw bytes over the padded struct.
    let n = d.len().min(size_of::<renddata>());
    ptr::copy_nonoverlapping(d.as_ptr(), (&raw mut (*p).RenderData).cast::<u8>(), n);
    0
});

handler!(TNEFVersion(t, id, d, size) {
    if d.len() != 4 {
        out(b"Incorrect size of version field, suspected corruption\n");
        return -1;
    }
    let minor = le16(&d[..2]);
    let major = le16(&d[2..]);
    set_cstr(&mut (*t).version, &format!("TNEF{major}.{minor}"));
    0
});

handler!(TNEFIcon(t, id, d, size) {
    let p = last_attachment(t);
    vl_free(&mut (*p).IconData);
    if !prealloc(size, 10_000) {
        return -1;
    }
    vl_set(&mut (*p).IconData, d, 0)
});

handler!(TNEFRecipTable(t, id, d, size) {
    if d.len() < 4 {
        return corrupted(line!());
    }
    0
});

handler!(TNEFAttachmentMAPI(t, id, d, size) {
    let p = last_attachment(t);
    fill_mapi(t, d, &raw mut (*p).MAPI)
});

handler!(TNEFMapiProperties(t, id, d, size) {
    if fill_mapi(t, d, &raw mut (*t).MapiProperties) < 0 {
        out(b"ERROR Parsing MAPI block\n");
        return -1;
    }
    if (*t).Debug >= 3 {
        MAPIPrint(&raw mut (*t).MapiProperties);
    }
    0
});

handler!(TNEFSentFor(t, id, d, size) {
    let mut rest = d;
    while !rest.is_empty() {
        let mut field = |line: u32| -> Result<&[u8], c_int> {
            let n = usize::from(le16(rest.get(..2).ok_or_else(|| corrupted(line))?));
            let v = rest.get(2..2 + n).ok_or_else(|| corrupted(line))?;
            rest = &rest[2 + n..];
            Ok(v)
        };
        let name = match field(line!()) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if (*t).Debug >= 1 {
            let mut s = b"Sent For : ".to_vec();
            s.extend_from_slice(upto_nul(name));
            out(&s);
        }
        let addr = match field(line!()) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if (*t).Debug >= 1 {
            let mut s = b"<".to_vec();
            s.extend_from_slice(upto_nul(addr));
            s.extend_from_slice(b">\n");
            out(&s);
        }
    }
    0
});

handler!(TNEFDateHandler(t, id, d, size) {
    if d.len() < size_of::<dtr>() {
        return -1;
    }
    let date: *mut dtr = match handler_entry(id).map(|h| h.id) {
        Some(att::DATE_SENT) => &raw mut (*t).dateSent,
        Some(att::DATE_RECD) => &raw mut (*t).dateReceived,
        Some(att::DATE_MODIFIED) => &raw mut (*t).dateModified,
        Some(att::DATE_START) => &raw mut (*t).DateStart,
        Some(att::DATE_END) => &raw mut (*t).DateEnd,
        Some(att::ATTACH_CREATE_DATE) => &raw mut (*last_attachment(t)).CreateDate,
        Some(att::ATTACH_MODIFY_DATE) => &raw mut (*last_attachment(t)).ModifyDate,
        _ => {
            if (*t).Debug >= 1 {
                out(b"MISSING CASE\n");
            }
            return YTNEF_UNKNOWN_PROPERTY;
        }
    };
    let w = |i: usize| le16(&d[2 * i..]);
    *date = dtr {
        wYear: w(0),
        wMonth: w(1),
        wDay: w(2),
        wHour: w(3),
        wMinute: w(4),
        wSecond: w(5),
        wDayOfWeek: w(6),
    };
    0
});

handler!(TNEFHexBreakdown(t, id, d, size) {
    if (*t).Debug == 0 {
        return 0;
    }
    let mut s = handler_name(id);
    s.extend_from_slice(format!(": [{size} bytes] \n").as_bytes());
    for (i, b) in d.iter().enumerate() {
        s.extend_from_slice(format!("{b:02x} ").as_bytes());
        if (i + 1) % 16 == 0 {
            s.push(b'\n');
        }
    }
    s.push(b'\n');
    out(&s);
    0
});

handler!(TNEFDetailedPrint(t, id, d, size) {
    if (*t).Debug == 0 {
        return 0;
    }
    let mut s = handler_name(id);
    s.extend_from_slice(format!(": [{size} bytes] \n").as_bytes());
    s.extend_from_slice(d);
    s.push(b'\n');
    out(&s);
    0
});

handler!(TNEFAttachmentFilename(t, id, d, size) {
    let p = last_attachment(t);
    vl_free(&mut (*p).Title);
    if !prealloc(size, 260) {
        return -1;
    }
    vl_set(&mut (*p).Title, d, 1)
});

handler!(TNEFAttachmentSave(t, id, d, size) {
    let p = last_attachment(t);
    vl_set(&mut (*p).FileData, d, 0)
});

handler!(TNEFPriority(t, id, d, size) {
    let s = match le32(d) {
        3 => "high",
        2 => "normal",
        1 => "low",
        _ => "N/A",
    };
    set_cstr(&mut (*t).priority, s);
    0
});

// ---------------------------------------------------------------------------
// MAPI property blocks

/// Print what C prints when a property block is malformed.
fn mapi_error(debug: c_int, e: &HandlerError, size: usize) {
    match e {
        HandlerError::TrailingBytes { len } => {
            if debug >= 1 {
                outf!(
                    "ERROR DURING MAPI READ\nRead {} bytes, Expected {} bytes\n{} bytes missing\n",
                    size - len,
                    size,
                    len
                );
            }
        }
        HandlerError::Overrun { extra } => {
            if debug >= 1 {
                outf!(
                    "ERROR DURING MAPI READ\nRead {} bytes, Expected {} bytes\n{} bytes extra\n",
                    size + extra,
                    size,
                    extra
                );
            }
        }
        HandlerError::BadPropType { .. } => out(b"Bad file\n"),
        HandlerError::OutOfRange { value, max, .. } => {
            prealloc(*value as c_int, *max);
        }
        HandlerError::TooLarge { len, max } => {
            prealloc(*len as c_int, *max);
        }
        _ => {
            corrupted(line!());
        }
    }
}

unsafe fn fill_mapi(t: *mut TNEFStruct, d: &[u8], p: *mut MAPIProps) -> c_int {
    TNEFFreeMapiProps(p);
    let debug = if t.is_null() { 0 } else { (*t).Debug };
    let props = match mapi::parse(d, &Limits::default()) {
        Ok(v) => v,
        Err(e) => {
            mapi_error(debug, &e, d.len());
            return -1;
        }
    };
    let arr = libc::calloc(props.len().max(1), size_of::<MAPIProperty>()).cast::<MAPIProperty>();
    if arr.is_null() {
        return oom(line!());
    }
    (*p).properties = arr;
    (*p).count = props.len() as DWORD;
    for (i, pr) in props.iter().enumerate() {
        let mp = &mut *arr.add(i);
        mp.id = (u32::from(pr.id()) << 16) | u32::from(pr.prop_type());
        if let Some(name) = &pr.name {
            mp.custom = 1;
            mp.guid = name.guid;
            if let NameKind::Strings(names) = &name.kind {
                let k = names.len();
                let pn =
                    libc::calloc(k.max(1), size_of::<variableLength>()).cast::<variableLength>();
                if pn.is_null() {
                    return oom(line!());
                }
                mp.propnames = pn;
                mp.namedproperty = k as c_int;
                // C keeps every other byte of the UTF-16 name, in reverse order.
                for (j, s) in names.iter().enumerate() {
                    let ascii: Vec<u8> = s.iter().step_by(2).take(s.len() / 2).copied().collect();
                    let buf = libc::calloc(s.len() + 1, 1).cast::<BYTE>();
                    if buf.is_null() {
                        return oom(line!());
                    }
                    ptr::copy_nonoverlapping(ascii.as_ptr(), buf, ascii.len());
                    *pn.add(k - 1 - j) = variableLength {
                        data: buf,
                        size: s.len() as c_int,
                    };
                }
            }
        }
        let m = pr.values.len();
        let vals = libc::calloc(m.max(1), size_of::<variableLength>()).cast::<variableLength>();
        if vals.is_null() {
            return oom(line!());
        }
        mp.data = vals;
        mp.count = m as ULONG;
        for (j, v) in pr.values.iter().enumerate() {
            let vl = &mut *vals.add(j);
            let data = match pr.prop_type() {
                pt::BINARY | pt::OBJECT | pt::STRING8 | pt::UNICODE if v.is_empty() => continue,
                pt::UNICODE => utf8_from_utf16(v),
                // one more byte so a PT_STRING8 without its NUL still ends
                pt::BINARY | pt::OBJECT | pt::STRING8 => dup(v, 1),
                // calloc(2, sizeof(WORD)) in C
                pt::I2 => dup(v, 2),
                _ => dup(v, 0),
            };
            if data.is_null() {
                return oom(line!());
            }
            vl.data = data;
            vl.size = v.len() as c_int;
        }
        if !t.is_null() && SUBJECT_IDS.contains(&pr.id()) {
            for j in 0..m {
                if (*t).subject.size != 0 {
                    break;
                }
                let vl = vals.add(j);
                if !prealloc((*vl).size, 1000) {
                    return -1;
                }
                if vl_set(&mut (*t).subject, vl_bytes(vl), 1) < 0 {
                    return -1;
                }
                let s = std::slice::from_raw_parts_mut((*t).subject.data, (*vl).size as usize);
                for c in s {
                    if matches!(*c, b'\\' | b'/' | 0) {
                        *c = b'_';
                    }
                }
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFillMapi(
    tnef: *mut TNEFStruct,
    data: *mut BYTE,
    size: DWORD,
    p: *mut MAPIProps,
) -> c_int {
    guard(-1, || unsafe {
        if p.is_null() || (data.is_null() && size > 0) {
            return -1;
        }
        let d = if data.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(data, size as usize)
        };
        fill_mapi(tnef, d, p)
    })
}

// ---------------------------------------------------------------------------
// init / free

#[no_mangle]
pub unsafe extern "C" fn TNEFInitMapi(p: *mut MAPIProps) {
    guard((), || unsafe {
        if let Some(p) = p.as_mut() {
            p.count = 0;
            p.properties = null_mut();
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFInitAttachment(p: *mut Attachment) {
    guard((), || unsafe {
        if !p.is_null() {
            // Every field's initial value is all-zero bits.
            ptr::write_bytes(p, 0, 1);
        }
    })
}

const NO_VL: variableLength = variableLength {
    data: null_mut(),
    size: 0,
};

#[no_mangle]
pub unsafe extern "C" fn TNEFInitialize(tnef: *mut TNEFStruct) {
    guard((), || unsafe {
        let Some(t) = tnef.as_mut() else { return };
        t.version = [0; 16];
        t.from = NO_VL;
        t.subject = NO_VL;
        t.dateSent = dtr::default();
        t.dateReceived = dtr::default();
        t.messageStatus = [0; 10];
        t.messageClass = [0; 50];
        t.messageID = [0; 50];
        t.parentID = [0; 50];
        t.conversationID = [0; 50];
        t.body = NO_VL;
        t.priority = [0; 10];
        TNEFInitAttachment(&raw mut t.starting_attach);
        t.dateModified = dtr::default();
        TNEFInitMapi(&raw mut t.MapiProperties);
        t.CodePage = NO_VL;
        t.OriginalMessageClass = NO_VL;
        t.Owner = NO_VL;
        t.SentFor = NO_VL;
        t.Delegate = NO_VL;
        t.DateStart = dtr::default();
        t.DateEnd = dtr::default();
        t.AidOwner = NO_VL;
        t.RequestRes = 0;
        // Debug is left alone, as in C.
        t.IO = TNEFIOStruct {
            InitProc: None,
            ReadProc: None,
            CloseProc: None,
            data: null_mut(),
        };
        t.attachmentSize = 50;
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFreeMapiProps(p: *mut MAPIProps) {
    guard((), || unsafe {
        let Some(p) = p.as_mut() else { return };
        if !p.properties.is_null() {
            for i in 0..p.count as usize {
                let mp = &mut *p.properties.add(i);
                if !mp.data.is_null() {
                    for j in 0..mp.count as usize {
                        vl_free(&mut *mp.data.add(j));
                    }
                    libc::free(mp.data.cast());
                }
                if !mp.propnames.is_null() {
                    for j in 0..mp.namedproperty.max(0) as usize {
                        vl_free(&mut *mp.propnames.add(j));
                    }
                    libc::free(mp.propnames.cast());
                }
            }
            libc::free(p.properties.cast());
        }
        p.properties = null_mut();
        p.count = 0;
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFreeAttachment(p: *mut Attachment) {
    guard((), || unsafe {
        let Some(p) = p.as_mut() else { return };
        vl_free(&mut p.Title);
        vl_free(&mut p.MetaFile);
        vl_free(&mut p.TransportFilename);
        vl_free(&mut p.FileData);
        vl_free(&mut p.IconData);
        TNEFFreeMapiProps(&raw mut p.MAPI);
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFree(tnef: *mut TNEFStruct) {
    guard((), || unsafe {
        let Some(t) = tnef.as_mut() else { return };
        for v in [
            &mut t.from,
            &mut t.subject,
            &mut t.body,
            &mut t.CodePage,
            &mut t.OriginalMessageClass,
            &mut t.Owner,
            &mut t.SentFor,
            &mut t.Delegate,
            &mut t.AidOwner,
        ] {
            vl_free(v);
        }
        TNEFFreeMapiProps(&raw mut t.MapiProperties);
        TNEFFreeAttachment(&raw mut t.starting_attach);
        let mut p = t.starting_attach.next;
        t.starting_attach.next = null_mut();
        while !p.is_null() {
            TNEFFreeAttachment(p);
            let next = (*p).next;
            libc::free(p.cast());
            p = next;
        }
    })
}

// ---------------------------------------------------------------------------
// IO

#[no_mangle]
pub unsafe extern "C" fn TNEFFile_Open(io: *mut TNEFIOStruct) -> c_int {
    guard(-1, || unsafe {
        let Some(f) = io
            .as_ref()
            .and_then(|io| io.data.cast::<TNEFFileInfo>().as_mut())
        else {
            return -1;
        };
        f.fptr = libc::fopen(f.filename, c"rb".as_ptr());
        if f.fptr.is_null() {
            -1
        } else {
            0
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFile_Read(
    io: *mut TNEFIOStruct,
    size: c_int,
    count: c_int,
    dest: *mut c_void,
) -> c_int {
    guard(-1, || unsafe {
        let Some(f) = io
            .as_ref()
            .and_then(|io| io.data.cast::<TNEFFileInfo>().as_mut())
        else {
            return -1;
        };
        if f.fptr.is_null() {
            return -1;
        }
        if size <= 0 || count <= 0 {
            return 0;
        }
        libc::fread(dest, size as usize, count as usize, f.fptr) as c_int
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFFile_Close(io: *mut TNEFIOStruct) -> c_int {
    guard(-1, || unsafe {
        if let Some(f) = io
            .as_ref()
            .and_then(|io| io.data.cast::<TNEFFileInfo>().as_mut())
        {
            if !f.fptr.is_null() {
                libc::fclose(f.fptr);
                f.fptr = null_mut();
            }
        }
        0
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFMemory_Open(io: *mut TNEFIOStruct) -> c_int {
    guard(-1, || unsafe {
        let Some(m) = io
            .as_ref()
            .and_then(|io| io.data.cast::<TNEFMemInfo>().as_mut())
        else {
            return -1;
        };
        m.ptr = m.dataStart;
        0
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFMemory_Read(
    io: *mut TNEFIOStruct,
    size: c_int,
    count: c_int,
    dest: *mut c_void,
) -> c_int {
    guard(-1, || unsafe {
        let Some(m) = io
            .as_ref()
            .and_then(|io| io.data.cast::<TNEFMemInfo>().as_mut())
        else {
            return -1;
        };
        let length = i64::from(size) * i64::from(count);
        let used = (m.ptr as isize).wrapping_sub(m.dataStart as isize) as i64;
        #[allow(clippy::useless_conversion)] // c_long is i32 on some targets
        let max = i64::from(m.size) - used;
        if length < 0 || length > max || (length > 0 && dest.is_null()) {
            return -1;
        }
        ptr::copy_nonoverlapping(m.ptr, dest.cast::<u8>(), length as usize);
        m.ptr = m.ptr.add(length as usize);
        count
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFMemory_Close(_io: *mut TNEFIOStruct) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn TNEFParseFile(filename: *mut c_char, tnef: *mut TNEFStruct) -> c_int {
    guard(-1, || unsafe {
        let Some(t) = tnef.as_mut() else { return -1 };
        if filename.is_null() {
            return YTNEF_CANNOT_INIT_DATA;
        }
        if t.Debug >= 1 {
            let mut s = b"Attempting to parse ".to_vec();
            s.extend_from_slice(std::ffi::CStr::from_ptr(filename).to_bytes());
            s.extend_from_slice(b"...\n");
            out(&s);
        }
        let mut finfo = TNEFFileInfo {
            filename,
            fptr: null_mut(),
            Debug: t.Debug,
        };
        t.IO.data = (&raw mut finfo).cast();
        t.IO.InitProc = Some(TNEFFile_Open);
        t.IO.ReadProc = Some(TNEFFile_Read);
        t.IO.CloseProc = Some(TNEFFile_Close);
        parse(tnef)
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFParseMemory(
    memory: *mut BYTE,
    size: c_long,
    tnef: *mut TNEFStruct,
) -> c_int {
    guard(-1, || unsafe {
        let Some(t) = tnef.as_mut() else { return -1 };
        if t.Debug >= 1 {
            outf!(
                "DEBUG(1/{}): Attempting to parse memory block...\n\n",
                t.Debug
            );
        }
        let mut minfo = TNEFMemInfo {
            dataStart: memory,
            ptr: memory,
            size,
            Debug: t.Debug,
        };
        t.IO.data = (&raw mut minfo).cast();
        t.IO.InitProc = Some(TNEFMemory_Open);
        t.IO.ReadProc = Some(TNEFMemory_Read);
        t.IO.CloseProc = Some(TNEFMemory_Close);
        parse(tnef)
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFParse(tnef: *mut TNEFStruct) -> c_int {
    guard(-1, || unsafe {
        if tnef.is_null() {
            return YTNEF_INCORRECT_SETUP;
        }
        parse(tnef)
    })
}

/// Read everything the caller's `ReadProc` yields, up to `max + 1` bytes.
/// Readers may be all-or-nothing (`TNEFMemory_Read` refuses a request
/// past the end), so a failed read is retried smaller before giving up.
unsafe fn drain(io: *mut TNEFIOStruct, read: ReadProc, max: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk: usize = 1 << 16;
    while buf.len() <= max {
        let old = buf.len();
        buf.resize(old + chunk, 0);
        let got = read(io, 1, chunk as c_int, buf.as_mut_ptr().add(old).cast());
        if got >= 1 {
            buf.truncate(old + (got as usize).min(chunk));
        } else {
            buf.truncate(old);
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    buf
}

/// Start of the attribute a stream error is in, when known.
fn error_start(e: &Error, buf: &[u8]) -> Option<usize> {
    match e {
        Error::InHandler { offset, .. }
        | Error::BadChecksum { offset, .. }
        | Error::EmptyAttribute { offset, .. } => Some(*offset),
        Error::Truncated { .. }
        | Error::NoChecksum { .. }
        | Error::LimitExceeded {
            what: "attribute size",
            ..
        } => {
            // the stream ends inside the last attribute: skip whole ones
            let mut pos = 6;
            while let Some(h) = buf.get(pos..pos + 9) {
                let end = (pos + 9)
                    .saturating_add(le32(&h[5..]) as usize)
                    .saturating_add(2);
                if end > buf.len() {
                    break;
                }
                pos = end;
            }
            Some(pos)
        }
        _ => None,
    }
}

/// Call the `TNEFList` handler for each attribute, as C's main loop does.
unsafe fn dispatch(tnef: *mut TNEFStruct, attrs: &[ytnef_core::Attribute]) -> c_int {
    let list = &raw const TNEFList;
    for a in attrs {
        for (i, h) in (*list).iter().enumerate() {
            if h.id != a.id {
                continue;
            }
            if let Some(f) = h.handler {
                // handlers get their own writable copy, as in C
                let mut data = a.data.to_vec();
                if f(
                    tnef,
                    i as c_int,
                    data.as_mut_ptr().cast(),
                    data.len() as c_int,
                ) < 0
                {
                    return YTNEF_ERROR_IN_HANDLER;
                }
                break;
            }
        }
    }
    0
}

unsafe fn parse(tnef: *mut TNEFStruct) -> c_int {
    let io = &raw mut (*tnef).IO;
    let Some(read) = (*io).ReadProc else {
        out(b"ERROR: Setup incorrectly: No ReadProc\n");
        return YTNEF_INCORRECT_SETUP;
    };
    if let Some(init) = (*io).InitProc {
        if init(io) != 0 {
            return YTNEF_CANNOT_INIT_DATA;
        }
    }
    let mut opts = Options::default();
    if let Ok(mb @ 1..) = usize::try_from((*tnef).attachmentSize) {
        opts.limits.max_attr_size = mb.saturating_mul(MIB);
    }
    let buf = drain(io, read, opts.limits.max_input);
    if let Some(close) = (*io).CloseProc {
        close(io);
    }
    let debug = (*tnef).Debug;
    match ytnef_core::parse_with(&buf, &opts) {
        Ok(t) => {
            let rc = dispatch(tnef, &t.attributes);
            if rc == 0 && debug >= 1 {
                // C reads a header past the last attribute and reports
                // which field of a partial one ran out.
                let end = t
                    .attributes
                    .last()
                    .map_or(6, |a| a.offset + 9 + a.data.len() + 2);
                match buf.len().saturating_sub(end) {
                    1..=4 => out(b"ERROR: Error reading type\n"),
                    5..=8 => out(b"ERROR: Error reading size\n"),
                    _ => {}
                }
            }
            rc
        }
        Err(e) => {
            // Handle the attributes before the bad one, like C does as it reads.
            if let Some(start) = error_start(&e, &buf) {
                if let Ok(t) = ytnef_core::parse_with(&buf[..start.min(buf.len())], &opts) {
                    if dispatch(tnef, &t.attributes) < 0 {
                        return YTNEF_ERROR_IN_HANDLER;
                    }
                }
            }
            report(tnef, debug, &e, &buf);
            e.code()
        }
    }
}

/// Print what C prints for a stream error.
unsafe fn report(tnef: *mut TNEFStruct, debug: c_int, e: &Error, buf: &[u8]) {
    match e {
        Error::NotTnefStream => out(b"ERROR: Signature does not match. Not TNEF.\n"),
        Error::NoSignature => out(b"ERROR: Error reading signature\n"),
        Error::NoKey => {
            if debug >= 1 {
                out(b"Error reading Key\n");
            }
            out(b"ERROR: Unable to retrieve key.\n");
        }
        Error::Truncated { .. } => {
            if debug >= 1 {
                out(b"ERROR: Error reading data\n");
            }
            out(b"ERROR: Unable to read data.\n");
        }
        Error::NoChecksum { .. } => {
            if debug >= 1 {
                out(b"ERROR: Error reading data\n");
            }
            out(b"ERROR: Unable to read checksum.\n");
        }
        Error::EmptyAttribute { .. } => out(b"ERROR: Field with size of 0\n"),
        Error::BadChecksum { .. } => out(b"ERROR: Checksum mismatch. Data corruption?:\n"),
        Error::InHandler { offset, .. } => {
            // Run the bad attribute through its handler for C's message.
            if let Some(h) = buf.get(*offset..offset.saturating_add(9)) {
                let len = le32(&h[5..]) as usize;
                if let Some(data) = buf.get(offset + 9..(offset + 9).saturating_add(len)) {
                    let a = ytnef_core::Attribute {
                        offset: *offset,
                        level: h[0],
                        id: le32(&h[1..5]),
                        data,
                        checksum_ok: true,
                    };
                    dispatch(tnef, &[a]);
                }
            }
        }
        Error::LimitExceeded { value, max, .. } => {
            prealloc((*value).min(u32::MAX as usize) as u32 as c_int, *max);
        }
        _ => {}
    }
}

#[no_mangle]
pub unsafe extern "C" fn TNEFGetKey(tnef: *mut TNEFStruct, key: *mut WORD) -> c_int {
    guard(YTNEF_ERROR_READING_DATA, || unsafe {
        let Some(t) = tnef.as_mut() else {
            return YTNEF_ERROR_READING_DATA;
        };
        let Some(read) = t.IO.ReadProc else {
            return YTNEF_ERROR_READING_DATA;
        };
        let mut b = [0u8; 2];
        if read(&raw mut t.IO, 2, 1, b.as_mut_ptr().cast()) < 1 {
            if t.Debug >= 1 {
                out(b"Error reading Key\n");
            }
            return YTNEF_ERROR_READING_DATA;
        }
        if !key.is_null() {
            *key = u16::from_le_bytes(b);
        }
        0
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFGetHeader(
    tnef: *mut TNEFStruct,
    typ: *mut DWORD,
    size: *mut DWORD,
) -> c_int {
    guard(YTNEF_ERROR_READING_DATA, || unsafe {
        let Some(t) = tnef.as_mut() else {
            return YTNEF_ERROR_READING_DATA;
        };
        let Some(read) = t.IO.ReadProc else {
            return YTNEF_ERROR_READING_DATA;
        };
        let io = &raw mut t.IO;
        let mut c = 0u8;
        if read(io, 1, 1, (&raw mut c).cast()) < 1 {
            return YTNEF_ERROR_READING_DATA;
        }
        let mut b = [0u8; 4];
        if read(io, 4, 1, b.as_mut_ptr().cast()) < 1 {
            if t.Debug >= 1 {
                out(b"ERROR: Error reading type\n");
            }
            return YTNEF_ERROR_READING_DATA;
        }
        let ty = u32::from_le_bytes(b);
        if read(io, 4, 1, b.as_mut_ptr().cast()) < 1 {
            if t.Debug >= 1 {
                out(b"ERROR: Error reading size\n");
            }
            return YTNEF_ERROR_READING_DATA;
        }
        if !typ.is_null() {
            *typ = ty;
        }
        if !size.is_null() {
            *size = u32::from_le_bytes(b);
        }
        0
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFRawRead(
    tnef: *mut TNEFStruct,
    data: *mut BYTE,
    size: DWORD,
    checksum: *mut WORD,
) -> c_int {
    guard(YTNEF_ERROR_READING_DATA, || unsafe {
        let Some(t) = tnef.as_mut() else {
            return YTNEF_ERROR_READING_DATA;
        };
        let Some(read) = t.IO.ReadProc else {
            return YTNEF_ERROR_READING_DATA;
        };
        let got = read(&raw mut t.IO, 1, size as c_int, data.cast());
        if got < 0 || (got as DWORD) < size {
            if t.Debug >= 1 {
                out(b"ERROR: Error reading data\n");
            }
            return YTNEF_ERROR_READING_DATA;
        }
        if !checksum.is_null() {
            *checksum = bytes(data.cast(), size as c_int)
                .iter()
                .fold(0u16, |s, &b| s.wrapping_add(u16::from(b)));
        }
        0
    })
}

#[no_mangle]
pub unsafe extern "C" fn TNEFCheckForSignature(sig: DWORD) -> c_int {
    if u32::from_le_bytes(sig.to_ne_bytes()) == ytnef_core::TNEF_SIGNATURE {
        0
    } else {
        YTNEF_NOT_TNEF_STREAM
    }
}

// ---------------------------------------------------------------------------
// lookups, dates, printing, RTF

unsafe fn find(p: *mut MAPIProps, id: c_uint, custom: DWORD) -> *mut variableLength {
    let Some(p) = p.as_ref() else {
        return MAPI_UNDEFINED;
    };
    if p.properties.is_null() {
        return MAPI_UNDEFINED;
    }
    for i in 0..p.count as usize {
        let mp = &*p.properties.add(i);
        if mp.id == id && mp.custom == custom {
            return mp.data;
        }
    }
    MAPI_UNDEFINED
}

#[no_mangle]
pub unsafe extern "C" fn MAPIFindUserProp(p: *mut MAPIProps, id: c_uint) -> *mut variableLength {
    guard(MAPI_UNDEFINED, || unsafe { find(p, id, 1) })
}

#[no_mangle]
pub unsafe extern "C" fn MAPIFindProperty(p: *mut MAPIProps, id: c_uint) -> *mut variableLength {
    guard(MAPI_UNDEFINED, || unsafe { find(p, id, 0) })
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// C `MAPISysTimetoDTRSize`, step for step.
fn systime(ticks: u64) -> dtr {
    let mut s = ticks / 10 / 1000 / 1000;
    let second = (s % 60) as u16;
    s /= 60;
    let minute = (s % 60) as u16;
    s /= 60;
    let hour = (s % 24) as u16;
    s /= 24;
    let dow = ((s + 1) % 7) as u16;
    let mut year: u32 = 1601;
    let mut days = 365;
    while s >= days {
        s -= days;
        year += 1;
        days = if is_leap(year) { 366 } else { 365 };
    }
    let mut months = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if is_leap(year) {
        months[1] = 29;
    }
    let mut month = 0;
    while month < 11 && s >= months[month] {
        s -= months[month];
        month += 1;
    }
    dtr {
        wYear: year as u16,
        wMonth: month as u16 + 1,
        wDay: s as u16 + 1,
        wHour: hour,
        wMinute: minute,
        wSecond: second,
        wDayOfWeek: dow,
    }
}

#[no_mangle]
pub unsafe extern "C" fn MAPISysTimetoDTR(data: *mut BYTE, thedate: *mut dtr) -> c_int {
    MAPISysTimetoDTRSize(data, 8, thedate)
}

#[no_mangle]
pub unsafe extern "C" fn MAPISysTimetoDTRSize(
    data: *mut BYTE,
    size: DWORD,
    thedate: *mut dtr,
) -> c_int {
    guard(-1, || unsafe {
        if thedate.is_null() {
            return -1;
        }
        *thedate = dtr::default();
        if data.is_null() || size < 8 {
            return -1;
        }
        *thedate = systime(le64(std::slice::from_raw_parts(data, 8)));
        0
    })
}

fn print_date(d: &dtr) -> String {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut s = String::new();
    if let Some(day) = DAYS.get(usize::from(d.wDayOfWeek)) {
        s += day;
        s += " ";
    }
    if (1..13).contains(&d.wMonth) {
        s += MONTHS[usize::from(d.wMonth) - 1];
        s += " ";
    }
    s += &format!("{}, {} ", d.wDay, d.wYear);
    s += &match d.wHour {
        h @ 13.. => format!("{}:{:02}:{:02} pm", h - 12, d.wMinute, d.wSecond),
        12 => format!("12:{:02}:{:02} pm", d.wMinute, d.wSecond),
        h => format!("{}:{:02}:{:02} am", h, d.wMinute, d.wSecond),
    };
    s
}

#[no_mangle]
pub extern "C" fn TNEFPrintDate(date: dtr) {
    guard((), || outf!("{}", print_date(&date)))
}

fn printable(b: &[u8]) -> Vec<u8> {
    b.iter()
        // SAFETY: isprint takes any unsigned char value.
        .map(|&c| {
            if unsafe { libc::isprint(c_int::from(c)) } != 0 {
                c
            } else {
                b'.'
            }
        })
        .collect()
}

fn type_label(t: u16) -> String {
    match t {
        PT_UNSPECIFIED => "  NONE   ".into(),
        PT_NULL => "  NULL   ".into(),
        pt::I2 => "   I2    ".into(),
        pt::LONG => "  LONG   ".into(),
        pt::R4 => "   R4    ".into(),
        pt::DOUBLE => " DOUBLE  ".into(),
        pt::CURRENCY => "CURRENCY ".into(),
        pt::APPTIME => "APP TIME ".into(),
        pt::ERROR => "  ERROR  ".into(),
        pt::BOOLEAN => " BOOLEAN ".into(),
        pt::OBJECT => " OBJECT  ".into(),
        pt::I8 => "   I8    ".into(),
        pt::STRING8 => " STRING8 ".into(),
        pt::UNICODE => " UNICODE ".into(),
        pt::SYSTIME => "SYS TIME ".into(),
        pt::CLSID => "OLE GUID ".into(),
        pt::BINARY => " BINARY  ".into(),
        other => format!("<{other:x}>"),
    }
}

unsafe fn mapi_print(p: &MAPIProps) {
    if p.properties.is_null() {
        return;
    }
    for j in 0..p.count as usize {
        let m = &*p.properties.add(j);
        let ty = (m.id & 0xffff) as u16;
        let pid = (m.id >> 16) as u16;
        let code = if m.custom == 1 {
            format!("UD:x{pid:04x}")
        } else {
            match MPList.iter().find(|e| e.id == c_uint::from(pid)) {
                Some(e) => String::from_utf8_lossy(upto_nul(&e.name.map(|c| c as u8))).into_owned(),
                None => format!("0x{pid:04x}"),
            }
        };
        outf!("   #{}: Type: [{}]  Code: [{}]\n", j, type_label(ty), code);
        if !m.propnames.is_null() {
            for i in 0..m.namedproperty.max(0) as usize {
                let n = &*m.propnames.add(i);
                let mut s = b"    Name: ".to_vec();
                if n.data.is_null() {
                    s.extend_from_slice(b"(null)");
                } else {
                    s.extend_from_slice(std::ffi::CStr::from_ptr(n.data.cast()).to_bytes());
                }
                s.push(b'\n');
                out(&s);
            }
        }
        if m.data.is_null() {
            continue;
        }
        for i in 0..m.count as usize {
            let v = m.data.add(i);
            let mut s = if m.count > 1 {
                format!("    [{}/{}] ", i, m.count).into_bytes()
            } else {
                b"    ".to_vec()
            };
            s.extend_from_slice(format!("Size: {}", (*v).size).as_bytes());
            if (*v).data.is_null() {
                s.extend_from_slice(if ty == pt::OBJECT {
                    b"\n"
                } else {
                    b"    Value: []\n"
                });
                out(&s);
                continue;
            }
            let d = vl_bytes(v);
            let hex = |n: usize| -> String {
                let mut a = vec![0u8; n];
                a[..d.len().min(n)].copy_from_slice(&d[..d.len().min(n)]);
                a.iter().map(|b| format!(" {b:02x}")).collect()
            };
            match ty {
                pt::SYSTIME => {
                    // a short value prints C's zeroed date
                    let date = if d.len() >= 8 {
                        systime(le64(d))
                    } else {
                        dtr::default()
                    };
                    s.extend_from_slice(
                        format!(
                            "    Value: {} [HEX: {}] ({})\n",
                            print_date(&date),
                            hex(8),
                            le64(d)
                        )
                        .as_bytes(),
                    );
                }
                pt::LONG => {
                    s.extend_from_slice(format!("    Value: {}\n", le32(d) as i32).as_bytes())
                }
                pt::I2 => {
                    s.extend_from_slice(format!("    Value: {}\n", le16(d) as i16).as_bytes())
                }
                pt::BOOLEAN => s.extend_from_slice(if d.first().copied().unwrap_or(0) != 0 {
                    b"    Value: True\n"
                } else {
                    b"    Value: False\n"
                }),
                pt::OBJECT => s.push(b'\n'),
                pt::BINARY => {
                    if is_compressed_rtf(d) {
                        s.extend_from_slice(
                            b"    Detected Compressed RTF. Decompressed text follows\n",
                        );
                        s.extend_from_slice(b"-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-\n");
                        out(&s);
                        s.clear();
                        if let Some(text) = decompress(d) {
                            s.extend_from_slice(upto_nul(&text));
                            s.push(b'\n');
                        }
                        s.extend_from_slice(b"-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-\n");
                    } else {
                        s.extend_from_slice(b"    Value: [");
                        s.extend_from_slice(&printable(d));
                        s.extend_from_slice(b"]\n");
                    }
                }
                pt::STRING8 => {
                    s.extend_from_slice(b"    Value: [");
                    s.extend_from_slice(upto_nul(d));
                    s.extend_from_slice(b"]\n");
                    if upto_nul(d).len() as i64 != i64::from((*v).size) - 1 {
                        s.extend_from_slice(b"Detected Hidden data: [");
                        s.extend_from_slice(&printable(d));
                        s.extend_from_slice(b"]\n");
                    }
                }
                pt::CLSID => {
                    s.extend_from_slice(format!("    Value: [HEX: {}]\n", hex(16)).as_bytes())
                }
                _ => {
                    s.extend_from_slice(b"    Value: [");
                    s.extend_from_slice(upto_nul(d));
                    s.extend_from_slice(b"]\n");
                }
            }
            out(&s);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn MAPIPrint(p: *mut MAPIProps) {
    guard((), || unsafe {
        if let Some(p) = p.as_ref() {
            mapi_print(p);
        }
    })
}

fn is_compressed_rtf(d: &[u8]) -> bool {
    d.len() >= 12
        && matches!(
            le32(&d[8..]),
            rtf::MAGIC_COMPRESSED | rtf::MAGIC_UNCOMPRESSED
        )
}

#[no_mangle]
pub unsafe extern "C" fn IsCompressedRTF(p: *mut variableLength) -> c_int {
    guard(0, || unsafe { c_int::from(is_compressed_rtf(vl_bytes(p))) })
}

/// Decompress, printing C's message on failure.
fn decompress(d: &[u8]) -> Option<Vec<u8>> {
    match rtf::decompress_rtf(d, &Limits::default()) {
        Ok(v) => Some(v),
        Err(e) => {
            match e {
                RtfError::TooShort { .. } => out(b"File too small\n"),
                RtfError::SizeMismatch { stored, len } => {
                    outf!(" Size Mismatch: {} != {}\n", stored, len as i64 - 4)
                }
                RtfError::Truncated { .. } => out(b"Corrupted stream\n"),
                RtfError::UnknownMagic { magic } => {
                    outf!("Unknown compression type (magic number {:x})\n", magic)
                }
                _ => out(b"Corrupted file\n"),
            }
            None
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn DecompressRTF(p: *mut variableLength, size: *mut c_int) -> *mut BYTE {
    guard(null_mut(), || unsafe {
        let Some(v) = decompress(vl_bytes(p)) else {
            return null_mut();
        };
        let r = dup(&v, 1);
        if r.is_null() {
            oom(line!());
            return r;
        }
        if !size.is_null() {
            *size = v.len() as c_int;
        }
        r
    })
}
