//! Call the exported functions the way C consumers do.

use libc::{c_int, c_void};
use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard};
use ytnef::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// `installed_handlers_are_called` patches the global `TNEFList`, which every
/// parse reads, so tests that parse take this lock.
fn tnef_list_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn new_tnef() -> Box<TNEFStruct> {
    let mut t: Box<MaybeUninit<TNEFStruct>> = Box::new_uninit();
    unsafe {
        std::ptr::write_bytes(t.as_mut_ptr(), 0, 1);
        TNEFInitialize(t.as_mut_ptr());
        t.assume_init()
    }
}

unsafe fn vl(v: &variableLength) -> &[u8] {
    if v.data.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(v.data, v.size as usize)
    }
}

fn cstr(a: &[libc::c_char]) -> String {
    a.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8 as char)
        .collect()
}

#[test]
fn parse_memory_fills_the_c_structs() {
    let _guard = tnef_list_lock();
    let mut buf = fixture("one-file.tnef");
    let mut t = new_tnef();
    unsafe {
        assert_eq!(
            TNEFParseMemory(buf.as_mut_ptr(), buf.len() as _, &mut *t),
            0
        );
        assert!(cstr(&t.version).starts_with("TNEF"));
        let a = t.starting_attach.next;
        assert!(!a.is_null(), "one attachment");
        assert!((*a).next.is_null());
        assert!(!vl(&(*a).Title).is_empty());
        // titles are NUL-terminated past their size
        assert_eq!(*(*a).Title.data.add((*a).Title.size as usize), 0);
        assert!(!vl(&(*a).FileData).is_empty());
        assert!((*a).MAPI.count > 0);
        // PR_ATTACH_LONG_FILENAME (PT_STRING8)
        let p = MAPIFindProperty(&mut (*a).MAPI, 0x3707_001E);
        if p != MAPI_UNDEFINED {
            assert!((*p).size > 0);
        }
        assert_eq!(
            MAPIFindProperty(&mut (*a).MAPI, 0x7FFF_0003),
            MAPI_UNDEFINED
        );
        TNEFFree(&mut *t);
        assert!(t.starting_attach.next.is_null());
        // a second free is harmless
        TNEFFree(&mut *t);
    }
}

#[test]
fn errors_match_tnef_errors_h() {
    let mut t = new_tnef();
    unsafe {
        let mut junk = b"not a tnef stream".to_vec();
        assert_eq!(
            TNEFParseMemory(junk.as_mut_ptr(), junk.len() as _, &mut *t),
            YTNEF_NOT_TNEF_STREAM
        );
        assert_eq!(
            TNEFParseMemory(junk.as_mut_ptr(), 0, &mut *t),
            YTNEF_ERROR_READING_DATA
        );
        TNEFInitialize(&mut *t);
        assert_eq!(TNEFParse(&mut *t), YTNEF_INCORRECT_SETUP);
        assert_eq!(TNEFParse(null_mut()), YTNEF_INCORRECT_SETUP);
        let mut name = *b"/nonexistent/winmail.dat\0";
        assert_eq!(
            TNEFParseFile(name.as_mut_ptr().cast(), &mut *t),
            YTNEF_CANNOT_INIT_DATA
        );
        TNEFFree(&mut *t);
    }
}

struct Source {
    data: Vec<u8>,
    pos: usize,
}

/// A ReadProc that hands out at most 3 bytes per call and refuses (-1) a
/// request it cannot fill completely.
unsafe extern "C" fn stingy_read(
    io: *mut TNEFIOStruct,
    size: c_int,
    count: c_int,
    dest: *mut c_void,
) -> c_int {
    let s = &mut *(*io).data.cast::<Source>();
    let want = (size * count) as usize;
    if want > s.data.len() - s.pos {
        return -1;
    }
    let n = (count as usize).min(3 / size.max(1) as usize).max(1);
    let bytes = n * size as usize;
    std::ptr::copy_nonoverlapping(s.data.as_ptr().add(s.pos), dest.cast(), bytes);
    s.pos += bytes;
    n as c_int
}

#[test]
fn custom_read_proc() {
    let _guard = tnef_list_lock();
    let data = fixture("winmail.dat");
    let mut reference = new_tnef();
    let mut t = new_tnef();
    let mut src = Source {
        data: data.clone(),
        pos: 0,
    };
    let mut copy = data.clone();
    unsafe {
        assert_eq!(
            TNEFParseMemory(copy.as_mut_ptr(), copy.len() as _, &mut *reference),
            0
        );
        t.IO.ReadProc = Some(stingy_read);
        t.IO.data = (&mut src as *mut Source).cast();
        assert_eq!(TNEFParse(&mut *t), 0);
        assert_eq!(src.pos, data.len());
        assert_eq!(vl(&t.subject), vl(&reference.subject));
        assert_eq!(cstr(&t.messageClass), cstr(&reference.messageClass));
        assert_eq!(t.MapiProperties.count, reference.MapiProperties.count);
        TNEFFree(&mut *t);
        TNEFFree(&mut *reference);
    }
}

#[test]
fn installed_handlers_are_called() {
    let _guard = tnef_list_lock();
    static CALLS: AtomicI32 = AtomicI32::new(0);
    unsafe extern "C" fn count(
        _: *mut TNEFStruct,
        _: c_int,
        _: *mut libc::c_char,
        _: c_int,
    ) -> c_int {
        CALLS.fetch_add(1, Ordering::Relaxed);
        0
    }
    let mut buf = fixture("one-file.tnef");
    let mut t = new_tnef();
    unsafe {
        let list = &raw mut TNEFList;
        let i = (*list).iter().position(|h| h.id == 0x0006_9002).unwrap();
        let saved = (*list)[i].handler;
        (*list)[i].handler = Some(count);
        let rc = TNEFParseMemory(buf.as_mut_ptr(), buf.len() as _, &mut *t);
        (*list)[i].handler = saved;
        assert_eq!(rc, 0);
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
        assert!(
            t.starting_attach.next.is_null(),
            "replaced handler made no attachment"
        );
        TNEFFree(&mut *t);
    }
}

#[test]
fn helpers() {
    let _guard = tnef_list_lock();
    unsafe {
        let mut b = [0x34, 0x12, 0x78, 0x56, 0, 0, 0, 0];
        assert_eq!(SwapWord(b.as_mut_ptr(), 2), 0x1234);
        assert_eq!(SwapWord(b.as_mut_ptr(), 1), 0x34);
        assert_eq!(SwapDWord(b.as_mut_ptr(), 4), 0x5678_1234);
        assert_eq!(SwapDDWord(b.as_mut_ptr(), 8), 0x5678_1234);
        assert_eq!(TNEFCheckForSignature(0x223E_9F78), 0);
        assert_eq!(TNEFCheckForSignature(0), YTNEF_NOT_TNEF_STREAM);

        // "é😀" plus an unpaired surrogate
        let mut u = [0xe9u8, 0x00, 0x3d, 0xd8, 0x00, 0xde, 0x00, 0xd8];
        let s = to_utf8(u.len(), u.as_mut_ptr().cast());
        assert_eq!(
            std::ffi::CStr::from_ptr(s).to_bytes(),
            "é😀\u{fffd}".as_bytes()
        );
        libc::free(s.cast());

        // 2009-02-13 23:31:30 UTC, a Friday
        let mut ft = (1_234_567_890u64 * 10_000_000 + 116_444_736_000_000_000).to_le_bytes();
        let mut d = dtr::default();
        assert_eq!(MAPISysTimetoDTR(ft.as_mut_ptr(), &mut d), 0);
        assert_eq!(
            (
                d.wYear,
                d.wMonth,
                d.wDay,
                d.wHour,
                d.wMinute,
                d.wSecond,
                d.wDayOfWeek
            ),
            (2009, 2, 13, 23, 31, 30, 5)
        );
        assert_eq!(MAPISysTimetoDTRSize(ft.as_mut_ptr(), 4, &mut d), -1);

        // uncompressed ("MELA") RTF
        let mut r = Vec::new();
        r.extend_from_slice(&(12u32 + 4).to_le_bytes());
        r.extend_from_slice(&4u32.to_le_bytes());
        r.extend_from_slice(&0x414C_454Du32.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.extend_from_slice(b"{\\rt");
        let mut v = variableLength {
            data: r.as_mut_ptr(),
            size: r.len() as c_int,
        };
        assert_eq!(IsCompressedRTF(&mut v), 1);
        let mut n = 0;
        let out = DecompressRTF(&mut v, &mut n);
        assert!(!out.is_null());
        assert_eq!(std::slice::from_raw_parts(out, n as usize + 1), b"{\\rt\0");
        libc::free(out.cast());

        assert_eq!(MAPIFindProperty(null_mut(), 0), MAPI_UNDEFINED);
        assert!(DecompressRTF(null_mut(), &mut n).is_null());
        TNEFFree(null_mut());
        TNEFFreeMapiProps(null_mut());
    }
}

#[test]
fn every_fixture_parses_without_panicking() {
    let _guard = tnef_list_lock();
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"),
        &mut files,
    );
    for f in files {
        let mut buf = std::fs::read(&f).unwrap();
        let mut t = new_tnef();
        unsafe {
            let rc = TNEFParseMemory(buf.as_mut_ptr(), buf.len() as _, &mut *t);
            assert!((-8..=0).contains(&rc), "{}: rc {rc}", f.display());
            TNEFFree(&mut *t);
        }
    }
}
