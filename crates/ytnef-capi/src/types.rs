//! `#[repr(C)]` mirrors of `tnef-types.h`, `ytnef.h` and `mapitags.h`.
//! Field names follow the C headers; `tests/layout.rs` checks sizes and
//! offsets against them.

#![allow(non_snake_case, non_camel_case_types)]

use libc::{c_char, c_int, c_long, c_uint, c_void, FILE};

pub type BYTE = u8;
pub type WORD = u16;
pub type DWORD = u32;
pub type ULONG = u32;
pub type DDWORD = u64;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct renddata {
    pub atyp: WORD,
    pub ulPosition: ULONG,
    pub dxWidth: WORD,
    pub dyHeight: WORD,
    pub dwFlags: DWORD,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct dtr {
    pub wYear: WORD,
    pub wMonth: WORD,
    pub wDay: WORD,
    pub wHour: WORD,
    pub wMinute: WORD,
    pub wSecond: WORD,
    pub wDayOfWeek: WORD,
}

#[repr(C)]
#[derive(Debug)]
pub struct variableLength {
    pub data: *mut BYTE,
    pub size: c_int,
}

#[repr(C)]
#[derive(Debug)]
pub struct MAPIProperty {
    pub custom: DWORD,
    pub guid: [BYTE; 16],
    pub id: DWORD,
    pub count: ULONG,
    pub namedproperty: c_int,
    pub propnames: *mut variableLength,
    pub data: *mut variableLength,
}

#[repr(C)]
#[derive(Debug)]
pub struct MAPIProps {
    pub count: DWORD,
    pub properties: *mut MAPIProperty,
}

#[repr(C)]
#[derive(Debug)]
pub struct Attachment {
    pub Date: dtr,
    pub Title: variableLength,
    pub MetaFile: variableLength,
    pub CreateDate: dtr,
    pub ModifyDate: dtr,
    pub TransportFilename: variableLength,
    pub RenderData: renddata,
    pub MAPI: MAPIProps,
    pub next: *mut Attachment,
    pub FileData: variableLength,
    pub IconData: variableLength,
}

pub type InitProc = unsafe extern "C" fn(*mut TNEFIOStruct) -> c_int;
pub type ReadProc = unsafe extern "C" fn(*mut TNEFIOStruct, c_int, c_int, *mut c_void) -> c_int;

#[repr(C)]
#[derive(Debug)]
pub struct TNEFIOStruct {
    pub InitProc: Option<InitProc>,
    pub ReadProc: Option<ReadProc>,
    pub CloseProc: Option<InitProc>,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Debug)]
pub struct TNEFFileInfo {
    pub filename: *mut c_char,
    pub fptr: *mut FILE,
    pub Debug: c_int,
}

#[repr(C)]
#[derive(Debug)]
pub struct TNEFMemInfo {
    pub dataStart: *mut BYTE,
    pub ptr: *mut BYTE,
    pub size: c_long,
    pub Debug: c_int,
}

#[repr(C)]
#[derive(Debug)]
pub struct TNEFStruct {
    pub version: [c_char; 16],
    pub from: variableLength,
    pub subject: variableLength,
    pub dateSent: dtr,
    pub dateReceived: dtr,
    pub messageStatus: [c_char; 10],
    pub messageClass: [c_char; 50],
    pub messageID: [c_char; 50],
    pub parentID: [c_char; 50],
    pub conversationID: [c_char; 50],
    pub body: variableLength,
    pub priority: [c_char; 10],
    pub starting_attach: Attachment,
    pub dateModified: dtr,
    pub MapiProperties: MAPIProps,
    pub CodePage: variableLength,
    pub OriginalMessageClass: variableLength,
    pub Owner: variableLength,
    pub SentFor: variableLength,
    pub Delegate: variableLength,
    pub DateStart: dtr,
    pub DateEnd: dtr,
    pub AidOwner: variableLength,
    pub RequestRes: c_int,
    pub Debug: c_int,
    pub IO: TNEFIOStruct,
    pub attachmentSize: c_int,
}

/// `STD_ARGLIST`: `(TNEFStruct *TNEF, int id, char *data, int size)`.
pub type Handler = unsafe extern "C" fn(*mut TNEFStruct, c_int, *mut c_char, c_int) -> c_int;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TNEFHandler {
    pub id: DWORD,
    pub name: [c_char; 40],
    pub handler: Option<Handler>,
}

#[repr(C)]
pub struct MAPIPropertyTagList {
    pub id: c_uint,
    pub name: [c_char; 128],
}
