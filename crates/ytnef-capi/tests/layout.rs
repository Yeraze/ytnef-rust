//! The `#[repr(C)]` mirrors must match the shipped C headers byte for byte:
//! compile a C program that prints every struct's size and every field's
//! offset, and compare with `size_of`/`offset_of!`. Skipped without a C
//! compiler (`$CC`, default `cc`).

use std::fmt::Write as _;
use std::mem::{offset_of, size_of};
use std::path::Path;
use std::process::Command;
use ytnef::*;

macro_rules! layout {
    ($($ty:ident { $($f:ident),* $(,)? })*) => {
        /// `(C expression, Rust value)` pairs.
        fn rust_layout() -> Vec<(String, usize)> {
            let mut v = Vec::new();
            $(
                v.push((format!("sizeof({})", stringify!($ty)), size_of::<$ty>()));
                $(v.push((
                    format!("offsetof({}, {})", stringify!($ty), stringify!($f)),
                    offset_of!($ty, $f),
                ));)*
            )*
            v
        }
    };
}

layout! {
    renddata { atyp, ulPosition, dxWidth, dyHeight, dwFlags }
    dtr { wYear, wMonth, wDay, wHour, wMinute, wSecond, wDayOfWeek }
    variableLength { data, size }
    MAPIProperty { custom, guid, id, count, namedproperty, propnames, data }
    MAPIProps { count, properties }
    Attachment {
        Date, Title, MetaFile, CreateDate, ModifyDate, TransportFilename,
        RenderData, MAPI, next, FileData, IconData,
    }
    TNEFIOStruct { InitProc, ReadProc, CloseProc, data }
    TNEFFileInfo { filename, fptr, Debug }
    TNEFMemInfo { dataStart, ptr, size, Debug }
    TNEFStruct {
        version, from, subject, dateSent, dateReceived, messageStatus,
        messageClass, messageID, parentID, conversationID, body, priority,
        starting_attach, dateModified, MapiProperties, CodePage,
        OriginalMessageClass, Owner, SentFor, Delegate, DateStart, DateEnd,
        AidOwner, RequestRes, Debug, IO, attachmentSize,
    }
    TNEFHandler { id, name, handler }
    MAPIPropertyTagList { id, name }
}

#[test]
fn layout_matches_c_headers() {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("layout");
    std::fs::create_dir_all(&dir).unwrap();
    let rust = rust_layout();

    let mut c = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"ytnef.h\"\n#include \"mapitags.h\"\nint main(void) {\n",
    );
    for (expr, _) in &rust {
        writeln!(c, "  printf(\"%zu\\n\", (size_t){expr});").unwrap();
    }
    c.push_str("  printf(\"%zu\\n\", sizeof(MPList) / sizeof(MPList[0]));\n");
    c.push_str("  return 0;\n}\n");
    let src = dir.join("layout.c");
    let exe = dir.join("layout");
    std::fs::write(&src, c).unwrap();

    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let built = Command::new(&cc)
        .arg("-I")
        .arg(&include)
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .status();
    match built {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("{cc} failed on {}: {s}", src.display()),
        Err(e) => {
            eprintln!("skipping: no C compiler ({cc}: {e})");
            return;
        }
    }
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success());
    let c_values: Vec<usize> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.parse().unwrap())
        .collect();

    let mut expected = rust.clone();
    expected.push(("MPList entries".into(), MPList.len()));
    assert_eq!(c_values.len(), expected.len());
    let bad: Vec<String> = expected
        .iter()
        .zip(&c_values)
        .filter(|((_, r), c)| r != *c)
        .map(|((e, r), c)| format!("{e}: C {c}, Rust {r}"))
        .collect();
    assert!(bad.is_empty(), "layout mismatch:\n{}", bad.join("\n"));
}
