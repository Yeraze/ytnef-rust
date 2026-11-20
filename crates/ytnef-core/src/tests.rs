#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use super::*;
use attr::id;

fn att(level: u8, id: u32, data: &[u8]) -> Vec<u8> {
    let mut v = vec![level];
    v.extend(id.to_le_bytes());
    v.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
    v.extend(data);
    let sum = data.iter().fold(0u16, |s, &b| s.wrapping_add(b.into()));
    v.extend(sum.to_le_bytes());
    v
}

fn msg(id: u32, data: &[u8]) -> Vec<u8> {
    att(attr::LVL_MESSAGE, id, data)
}

fn stream(attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut v = TNEF_SIGNATURE.to_le_bytes().to_vec();
    v.extend(0x1234u16.to_le_bytes());
    attrs.iter().for_each(|a| v.extend(a));
    v
}

fn ok(attrs: &[Vec<u8>]) -> Tnef<'static> {
    let buf = stream(attrs).leak();
    parse(buf, &Limits::default()).unwrap()
}

fn err(attrs: &[Vec<u8>]) -> Error {
    parse(&stream(attrs), &Limits::default()).unwrap_err()
}

fn handler_err(attrs: &[Vec<u8>]) -> HandlerError {
    match err(attrs) {
        e @ Error::InHandler { .. } => {
            assert_eq!(e.code(), -6);
            let Error::InHandler { reason, .. } = e else {
                unreachable!()
            };
            reason
        }
        e => panic!("expected handler error, got {e:?}"),
    }
}

const DATE: [u8; 14] = [0xD3, 0x07, 2, 0, 1, 0, 13, 0, 5, 0, 9, 0, 6, 0];
const DTR: Dtr = Dtr {
    year: 2003,
    month: 2,
    day: 1,
    hour: 13,
    minute: 5,
    second: 9,
    day_of_week: 6,
};

#[test]
fn signature() {
    assert!(has_signature(&[0x78, 0x9F, 0x3E, 0x22, 0x00]));
    assert!(!has_signature(&[0x78, 0x9F, 0x3E]));
    assert!(!has_signature(b"PK\x03\x04"));
}

#[test]
fn stream_framing() {
    let l = Limits::default();
    assert_eq!(parse(b"\x78\x9f", &l), Err(Error::NoSignature));
    assert_eq!(parse(b"PK\x03\x04\0\0", &l), Err(Error::NotTnefStream));
    assert_eq!(parse(&stream(&[])[..5], &l), Err(Error::NoKey));
    let t = ok(&[]);
    assert_eq!(t.key, 0x1234);
    assert!(t.attributes.is_empty());
    // fewer than 9 trailing bytes end the stream quietly, as in C
    let t = ok(&[msg(id::BODY, b"x"), vec![1, 2, 3, 4, 5, 6, 7, 8]]);
    assert_eq!(t.body, Some(&b"x"[..]));
}

#[test]
fn attribute_framing_errors() {
    let mut zero = msg(id::BODY, b"");
    zero.truncate(9);
    assert!(matches!(
        err(&[zero]),
        Error::EmptyAttribute { offset: 6, .. }
    ));

    let mut short = msg(id::BODY, b"hello");
    short.truncate(12);
    let e = err(&[short]);
    assert!(matches!(e, Error::Truncated { .. }));
    assert_eq!(e.code(), -3);

    let mut no_sum = msg(id::BODY, b"hello");
    no_sum.pop();
    assert_eq!(err(&[no_sum]).code(), -3);

    let huge = [msg(id::ATTACH_DATA, &[0; 64])];
    let l = Limits {
        max_attr_size: 63,
        ..Limits::default()
    };
    let e = parse(&stream(&huge), &l).unwrap_err();
    assert!(matches!(
        e,
        Error::LimitExceeded {
            value: 64,
            max: 63,
            ..
        }
    ));
    assert_eq!(e.code(), -1);
}

#[test]
fn checksum_strict_and_lenient() {
    let mut bad = msg(id::SUBJECT, b"hi");
    *bad.last_mut().unwrap() ^= 1;
    let buf = stream(&[bad]);
    let e = parse(&buf, &Limits::default()).unwrap_err();
    assert!(matches!(
        e,
        Error::BadChecksum {
            attr: id::SUBJECT,
            offset: 6,
            ..
        }
    ));
    assert_eq!(e.code(), -5);

    let lenient = Options {
        checksum: Checksum::Lenient,
        ..Options::default()
    };
    let t = parse_with(&buf, &lenient).unwrap();
    assert_eq!(t.subject, Some(&b"hi"[..]));
    assert!(!t.attributes[0].checksum_ok);
}

#[test]
fn capped_strings() {
    let l = Limits::default();
    for (att, max) in [
        (id::FROM, l.max_from),
        (id::SUBJECT, l.max_subject),
        (id::BODY, l.max_body),
        (id::ORIGINAL_MESSAGE_CLASS, l.max_original_message_class),
        (id::ATTACH_TITLE, l.max_title),
        (id::ATTACH_META_FILE, l.max_icon),
    ] {
        ok(&[msg(att, &vec![b'a'; max])]);
        assert_eq!(
            handler_err(&[msg(att, &vec![b'a'; max + 1])]),
            HandlerError::TooLarge { len: max + 1, max },
            "{att:#x}"
        );
    }
}

#[test]
fn message_strings() {
    let t = ok(&[
        msg(id::FROM, b"from"),
        msg(id::SUBJECT, b"subj"),
        msg(id::BODY, b"body"),
        msg(id::MESSAGE_CLASS, b"IPM.Note\0"),
        msg(id::MESSAGE_ID, b"mid"),
        msg(id::PARENT_ID, b"pid"),
        msg(id::ORIGINAL_MESSAGE_CLASS, b"orig"),
        msg(id::OEM_CODEPAGE, &[0xE4, 0x04, 0, 0]),
        msg(id::MAPI_PROPS, &[1, 0, 0, 0, 3, 0, 7, 0x0E, 5, 0, 0, 0]),
    ]);
    assert_eq!(t.from, Some(&b"from"[..]));
    assert_eq!(t.subject, Some(&b"subj"[..]));
    assert_eq!(t.body, Some(&b"body"[..]));
    assert_eq!(t.message_class, Some(&b"IPM.Note\0"[..]));
    assert_eq!(t.message_id, Some(&b"mid"[..]));
    assert_eq!(t.parent_id, Some(&b"pid"[..]));
    assert_eq!(t.original_message_class, Some(&b"orig"[..]));
    assert_eq!(t.oem_codepage, Some(&[0xE4, 0x04, 0, 0][..]));
    assert_eq!(t.mapi.len(), 1);
    assert_eq!(t.mapi[0].values, [&[5, 0, 0, 0][..]]);
}

#[test]
fn repeated_attribute_last_wins() {
    let t = ok(&[msg(id::SUBJECT, b"one"), msg(id::SUBJECT, b"two")]);
    assert_eq!(t.subject, Some(&b"two"[..]));
    assert_eq!(t.attributes.len(), 2);
}

#[test]
fn dates() {
    let t = ok(&[
        msg(id::DATE_SENT, &DATE),
        msg(id::DATE_RECD, &DATE),
        msg(id::DATE_MODIFIED, &DATE),
        msg(id::DATE_START, &DATE),
        msg(id::DATE_END, &[DATE.as_slice(), b"extra"].concat()),
    ]);
    for d in [
        t.date_sent,
        t.date_received,
        t.date_modified,
        t.date_start,
        t.date_end,
    ] {
        assert_eq!(d, Some(DTR));
    }
    for att in [
        id::DATE_SENT,
        id::ATTACH_CREATE_DATE,
        id::ATTACH_MODIFY_DATE,
    ] {
        assert_eq!(
            handler_err(&[msg(att, &DATE[..13])]),
            HandlerError::BadLength { len: 13 }
        );
    }
}

#[test]
fn version() {
    assert_eq!(
        ok(&[msg(id::TNEF_VERSION, &[0, 0, 1, 0])]).version,
        Some((1, 0))
    );
    assert_eq!(
        handler_err(&[msg(id::TNEF_VERSION, &[0, 0])]),
        HandlerError::BadLength { len: 2 }
    );
    assert_eq!(
        handler_err(&[msg(id::TNEF_VERSION, &[0; 5])]),
        HandlerError::BadLength { len: 5 }
    );
}

#[test]
fn priority() {
    assert_eq!(ok(&[msg(id::PRIORITY, &[3])]).priority, Some(3));
    assert_eq!(ok(&[msg(id::PRIORITY, &[2, 0])]).priority, Some(2));
    assert_eq!(ok(&[msg(id::PRIORITY, &[1, 0, 0, 0, 9])]).priority, Some(1));
}

#[test]
fn recip_table() {
    ok(&[msg(id::RECIP_TABLE, &[5, 0, 0, 0])]);
    assert!(matches!(
        handler_err(&[msg(id::RECIP_TABLE, &[5, 0, 0])]),
        HandlerError::Truncated { .. }
    ));
}

#[test]
fn sent_for() {
    ok(&[msg(id::SENT_FOR, b"\x02\0ab\x03\0xyz")]);
    ok(&[msg(id::SENT_FOR, b"\0\0\0\0")]);
    assert!(matches!(
        handler_err(&[msg(id::SENT_FOR, b"\x02\0ab\x05\0xyz")]),
        HandlerError::Truncated { .. }
    ));
    assert!(matches!(
        handler_err(&[msg(id::SENT_FOR, b"\x01")]),
        HandlerError::Truncated { .. }
    ));
}

#[test]
fn attachments() {
    let a = |id, d: &[u8]| att(attr::LVL_ATTACHMENT, id, d);
    let rend = [1, 0, 0xff, 0xff, 0xff, 0xff, 0x20, 0, 0x10, 0, 1, 0, 0, 0];
    let t = ok(&[
        a(id::ATTACH_TITLE, b"orphan"),
        a(id::ATTACH_RENDDATA, &rend),
        a(id::ATTACH_TITLE, b"one.txt"),
        a(id::ATTACH_DATA, b"payload"),
        a(id::ATTACH_META_FILE, b"icon"),
        a(id::ATTACH_CREATE_DATE, &DATE),
        a(id::ATTACH_MODIFY_DATE, &DATE),
        a(id::ATTACHMENT, &[1, 0, 0, 0, 3, 0, 7, 0x0E, 5, 0, 0, 0]),
        a(id::ATTACH_RENDDATA, &[2]),
        a(id::ATTACH_DATA, b"two"),
    ]);
    assert_eq!(t.head.title, Some(&b"orphan"[..]));
    assert_eq!(t.attachments.len(), 2);
    let one = &t.attachments[0];
    assert_eq!(
        one.render,
        RendData {
            atyp: 1,
            position: u32::MAX,
            width: 32,
            height: 16,
            flags: 1
        }
    );
    assert_eq!(one.title, Some(&b"one.txt"[..]));
    assert_eq!(one.data, Some(&b"payload"[..]));
    assert_eq!(one.icon, Some(&b"icon"[..]));
    assert_eq!(one.create_date, Some(DTR));
    assert_eq!(one.modify_date, Some(DTR));
    assert_eq!(one.mapi[0].id(), 0x0E07);
    // short renddata is zero-filled
    assert_eq!(
        t.attachments[1].render,
        RendData {
            atyp: 2,
            ..RendData::default()
        }
    );
    assert_eq!(t.attachments[1].data, Some(&b"two"[..]));
}

#[test]
fn unhandled_attributes_are_kept() {
    let t = ok(&[
        msg(id::OWNER, b"o"),
        msg(id::CONVERSATION_ID, b"c"),
        msg(0x0042_4242, b"?"),
    ]);
    let ids: Vec<u32> = t.attributes.iter().map(|a| a.id).collect();
    assert_eq!(ids, [id::OWNER, id::CONVERSATION_ID, 0x0042_4242]);
    assert_eq!(t.attributes[2].offset, 6 + 2 * 12);
}

#[test]
fn count_and_size_limits() {
    let rend = att(attr::LVL_ATTACHMENT, id::ATTACH_RENDDATA, &[1]);
    let buf = stream(&[rend.clone(), rend.clone(), rend]);
    let l = |f: fn(&mut Limits)| {
        let mut l = Limits::default();
        f(&mut l);
        l
    };
    assert!(parse(&buf, &l(|l| l.max_attachments = 3)).is_ok());
    assert!(matches!(
        parse(&buf, &l(|l| l.max_attachments = 2)),
        Err(Error::LimitExceeded {
            what: "attachment count",
            ..
        })
    ));
    assert!(matches!(
        parse(&buf, &l(|l| l.max_attributes = 2)),
        Err(Error::LimitExceeded {
            what: "attribute count",
            ..
        })
    ));
    assert!(matches!(
        parse(&buf, &l(|l| l.max_input = 10)),
        Err(Error::LimitExceeded {
            what: "input size",
            ..
        })
    ));
}

#[test]
fn errors_display() {
    let e = Error::InHandler {
        attr: id::BODY,
        offset: 6,
        reason: HandlerError::TooLarge { len: 9, max: 8 },
    };
    assert_eq!(
        e.to_string(),
        "attribute 0x0002800c at offset 6: payload length 9 exceeds limit 8"
    );
}

/// One-property MAPI block: PR_SUBJECT as PT_STRING8.
fn mapi_subject(s: &[u8]) -> Vec<u8> {
    let mut v = [1u32, 0x0037_001E, 1, u32::try_from(s.len()).unwrap()]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect::<Vec<u8>>();
    v.extend(s);
    v.resize(v.len() + (4 - s.len() % 4) % 4, 0);
    v
}

#[test]
fn mapi_subject_fills_missing_subject() {
    let t = ok(&[msg(id::MAPI_PROPS, &mapi_subject(b"from mapi"))]);
    assert_eq!(t.subject, Some(&b"from mapi"[..]));
    assert_eq!(t.mapi[0].values, [&b"from mapi"[..]]);
    // attSubject first: MAPI does not replace it
    let t = ok(&[
        msg(id::SUBJECT, b"att"),
        msg(id::MAPI_PROPS, &mapi_subject(b"from mapi")),
    ]);
    assert_eq!(t.subject, Some(&b"att"[..]));
    // attSubject later replaces it
    let t = ok(&[
        msg(id::MAPI_PROPS, &mapi_subject(b"from mapi")),
        msg(id::SUBJECT, b"att"),
    ]);
    assert_eq!(t.subject, Some(&b"att"[..]));
    // attachment MAPI fills the message subject too, as in C
    let t = ok(&[att(
        attr::LVL_ATTACHMENT,
        id::ATTACHMENT,
        &mapi_subject(b"x"),
    )]);
    assert_eq!(t.subject, Some(&b"x"[..]));
}

#[test]
fn bad_mapi_is_a_handler_error() {
    for (attr_id, data) in [
        (id::MAPI_PROPS, vec![0, 0, 0, 0]),
        (id::MAPI_PROPS, mapi_subject(b"")),
        (id::MAPI_PROPS, mapi_subject(&[b'a'; 1001])),
        (id::ATTACHMENT, vec![1, 0, 0, 0, 0x99, 0, 0, 0]),
    ] {
        let e = err(&[msg(attr_id, &data)]);
        assert!(
            matches!(e, Error::InHandler { attr, .. } if attr == attr_id),
            "{e}"
        );
        assert_eq!(e.code(), -6);
    }
}
