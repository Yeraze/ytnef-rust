#!/usr/bin/env python3
"""Write the synthetic contact / task / calendar fixtures in
tests/fixtures/export/. Deterministic: rerunning gives identical files.

    scripts/gen-export-fixtures.py
"""
import calendar
import os
import struct

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tests", "fixtures", "export")

# attribute IDs
SUBJECT, MESSAGE_ID, BODY = 0x00018004, 0x00018009, 0x0002800C
MESSAGE_CLASS, MAPI_PROPS = 0x00078008, 0x00069003

# property types
I2, LONG, BOOLEAN, STRING8, UNICODE, SYSTIME, BINARY = 0x2, 0x3, 0xB, 0x1E, 0x1F, 0x40, 0x102

# PS_PUBLIC_STRINGS-like GUID; ytnef ignores it
GUID = bytes(range(16))


def attr(id_, data):
    return struct.pack("<BII", 1, id_, len(data)) + data + struct.pack("<H", sum(data) & 0xFFFF)


def pad(b):
    return b + b"\0" * (-len(b) % 4)


def value(ty, v):
    if ty in (STRING8, UNICODE, BINARY):
        return struct.pack("<II", 1, len(v)) + pad(v)
    if ty == I2:
        return struct.pack("<hH", v, 0)
    if ty in (LONG, BOOLEAN):
        return struct.pack("<i", v)
    if ty == SYSTIME:
        return struct.pack("<Q", v)
    raise ValueError(ty)


def mapi(props):
    """props: (type, id, value, named). Named properties get stored IDs
    from 0x8000 up and a numeric name carrying `id`."""
    out = struct.pack("<I", len(props))
    stored = 0x8000
    for ty, id_, v, named in props:
        if named:
            out += struct.pack("<I", stored << 16 | ty) + GUID + struct.pack("<II", 0, id_)
            stored += 1
        else:
            out += struct.pack("<I", id_ << 16 | ty)
        out += value(ty, v)
    return out


def p(ty, id_, v):
    return (ty, id_, v, False)


def n(ty, id_, v):
    return (ty, id_, v, True)


def s8(s):
    return s.encode() + b"\0"


def u16(s):
    return s.encode("utf-16-le") + b"\0\0"


def ft(y, mo, d, h=0, mi=0, s=0):
    unix = calendar.timegm((y, mo, d, h, mi, s, 0, 0, 0))
    return (unix + 11644473600) * 10_000_000


def rtf(text):
    """Uncompressed ("MELA") RTF property value."""
    raw = text.encode()
    return struct.pack("<IIII", len(raw) + 12, len(raw), 0x414C454D, 0) + raw


def tnef(cls, props, subject=None, body=None, message_id=None):
    out = struct.pack("<IH", 0x223E9F78, 0x0101)
    if subject is not None:
        out += attr(SUBJECT, subject)
    if message_id is not None:
        out += attr(MESSAGE_ID, message_id)
    out += attr(MESSAGE_CLASS, s8(cls))
    if body is not None:
        out += attr(BODY, body)
    return out + attr(MAPI_PROPS, mapi(props))


def recur(kind, pattern=0, at=None, size=0x28):
    """A recurrence blob of `size` bytes: `kind` at 0x04, `pattern` at
    0x06, then the bytes in `at` (offset -> value)."""
    b = bytearray(size)
    b[0x04], b[0x06] = kind, pattern
    for k, v in (at or {}).items():
        b[k] = v
    return bytes(b)


def appointment(extra, cls="IPM.Appointment"):
    return tnef(cls, [
        p(STRING8, 0x0070, s8("Recurring")),
        n(SYSTIME, 0x820D, ft(2026, 3, 2, 14)),
        n(SYSTIME, 0x820E, ft(2026, 3, 2, 15)),
    ] + extra)


FIXTURES = {
    "contact.tnef": tnef("IPM.Contact", [
        p(STRING8, 0x3001, s8("Ada Lovelace")),
        p(STRING8, 0x3A4F, s8("Ada")),
        n(STRING8, 0x8554, s8("16.0")),
        p(STRING8, 0x3A48, s8("William")),
        p(STRING8, 0x3A16, s8("Analytical Engines")),
        p(STRING8, 0x3A18, s8("Research")),
        p(STRING8, 0x3A17, s8("Programmer")),
        p(STRING8, 0x1000, s8("plain note")),
        p(STRING8, 0x3A29, s8("1 Engine Way\r\nUnit 2")),
        p(STRING8, 0x3A27, s8("London")),
        p(STRING8, 0x3A2A, s8("N1 9GU")),
        p(STRING8, 0x3A26, s8("UK")),
        n(STRING8, 0x801B, s8("1 Engine Way\nLondon")),
        p(STRING8, 0x3A59, s8("Marylebone")),
        n(STRING8, 0x801A, s8("Home label")),
        p(STRING8, 0x3A5F, s8("Elsewhere")),
        p(STRING8, 0x3A08, s8("+44 20 0000 0001")),
        p(STRING8, 0x3A1C, s8("+44 7700 000002")),
        p(STRING8, 0x3A24, s8("+44 20 0000 0003")),
        n(STRING8, 0x8083, s8("ada@engines.test")),
        n(STRING8, 0x8094, s8("ada.home@example.test")),
        p(STRING8, 0x3A51, s8("http://engines.test/")),
        n(STRING8, 0x80D8, s8("http://engines.test/fb")),
        p(SYSTIME, 0x3A42, ft(1815, 12, 10)),
        p(SYSTIME, 0x3A41, ft(1835, 7, 8)),
    ], body=b"line one\r\nline two\0"),
    # no display name: the company names the card
    "contact-company.tnef": tnef("IPM.Contact", [
        p(STRING8, 0x3A16, s8("Acme / Sons")),
        p(STRING8, 0x3A2D, s8("12345")),
    ]),
    # no names or subject at all
    "contact-unknown.tnef": tnef("IPM.Contact", [
        p(STRING8, 0x3A4F, s8("nobody")),
    ]),
    "task.tnef": tnef("IPM.Task", [
        p(STRING8, 0x0070, s8("Write report ")),
        n(STRING8, 0x8122, s8("boss@example.test")),
        p(STRING8, 0x0E04, s8("Ann; Bob")),
        n(STRING8, 0x811F, s8("ann@example.test; bob@example.test;")),
        p(SYSTIME, 0x3007, ft(2026, 1, 5, 9, 30, 15)),
        n(SYSTIME, 0x8517, ft(2026, 1, 9, 17)),
        p(SYSTIME, 0x3008, ft(2026, 1, 6, 8, 1, 2)),
        n(BOOLEAN, 0x8506, 1),
    ], subject=b"Write report, final; v2\0", message_id=b"0123456789ABCDEF\0",
        body=b"Line 1\r\nsemi; comma, back\\slash\0"),
    # no PR_CONVERSATION_TOPIC: C writes no task
    "task-no-topic.tnef": tnef("IPM.Task", [
        n(BOOLEAN, 0x8506, 0),
    ], subject=b"untitled\0"),
    "appointment.tnef": tnef("IPM.Appointment", [
        n(BINARY, 0x0003, bytes([0x04, 0x00, 0x00, 0x00, 0x82, 0x00, 0xE0, 0x00, 0xAB, 0xCD])),
        n(LONG, 0x8201, 3),
        p(BINARY, 0x0C1D, b"SMTP:boss@example.test\0"),
        n(STRING8, 0x823B, s8("ann@example.test; bob@example.test")),
        n(STRING8, 0x823C, s8("carol@example.test")),
        p(STRING8, 0x0070, s8("Planning, round 2")),
        p(BINARY, 0x1009, rtf(
            "{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}\\pard\\f0 Agenda: one, two; three\\par\r\n"
            "{\\listtext 1.}see {\\field{\\*\\fldinst{HYPERLINK \"http://example.test/a\"}}"
            "{\\fldrslt{\\ul\\cf1 the plan}}}\\~now\\par\r\n}")),
        n(STRING8, 0x0002, s8("Room 4")),
        n(SYSTIME, 0x820D, ft(2026, 2, 3, 14)),
        n(SYSTIME, 0x820E, ft(2026, 2, 3, 15, 30)),
        n(SYSTIME, 0x8202, ft(2026, 1, 20, 12)),
        n(BOOLEAN, 0x8506, 0),
        n(BINARY, 0x8216, recur(0x0B, at={0x0E: 2, 0x16: 0x2A, 0x1A: 0x22, 0x1E: 10})),
    ]),
    # Unicode values, the 0x8238 attendee list, no recurrence
    "mtgreq.tnef": tnef("IPM.Microsoft Schedule.MtgReq", [
        n(BINARY, 0x0023, bytes([0x01, 0x02, 0xFE])),
        p(UNICODE, 0x0065, u16("organizer@example.test")),
        n(UNICODE, 0x8238, u16("dan@example.test;  eve@example.test")),
        p(UNICODE, 0x0070, u16("Sync, weekly")),
        p(BINARY, 0x1009, rtf("{\\rtf1 no paragraph marker}")),
        n(UNICODE, 0x8208, u16("Café B")),
        n(SYSTIME, 0x8516, ft(2026, 4, 1, 9)),
        n(SYSTIME, 0x8517, ft(2026, 4, 1, 9, 45)),
        n(SYSTIME, 0x001A, ft(2026, 3, 30)),
        n(BOOLEAN, 0x8506, 1),
    ]),
    # flagged an appointment by the named 0x24 property; class ends .MtgCncl
    "mtgcncl.tnef": tnef("IPM.Microsoft Schedule.MtgCncl", [
        n(STRING8, 0x0024, s8("IPM.Appointment")),
        p(STRING8, 0x0070, s8("Cancelled")),
        p(BINARY, 0x1009, b"\x10\0\0\0\x20\0\0\0LZFu\0\0\0\0truncated"),
    ]),
    "recur-daily.tnef": appointment([
        n(I2, 0x0011, 3),
        n(BINARY, 0x8216, recur(0x0A, at={0x16: 0x22, 0x1A: 7})),
    ]),
    "recur-daily-open.tnef": appointment([
        n(I2, 0x0011, 1),
        n(BINARY, 0x8216, recur(0x0A, at={0x16: 0x23})),
    ]),
    "recur-weekdays.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0A, at={0x16: 0x3E, 0x1A: 0x21, 0x1E: 20})),
    ]),
    "recur-monthly.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0C, 0x02, {0x0E: 1, 0x16: 15, 0x1A: 0x22, 0x1E: 12})),
    ]),
    "recur-monthly-nth.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0C, 0x03, {0x0E: 2, 0x16: 0x04, 0x1A: 0x05, 0x1E: 0x22, 0x22: 6})),
    ]),
    "recur-yearly.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0D, 0x02, {0x0A: 0x60, 0x0B: 0x51, 0x16: 21})),
    ]),
    "recur-yearly-nth.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0D, 0x03, {0x0A: 0xE0, 0x0B: 0x56, 0x16: 0x02, 0x1A: 1, 0x1E: 0x21, 0x22: 5})),
    ]),
    # shorter than 0x1F bytes: no RRULE line
    "recur-short.tnef": appointment([
        n(BINARY, 0x8216, recur(0x0B, size=0x1E)),
    ]),
}

os.makedirs(OUT, exist_ok=True)
for name, data in FIXTURES.items():
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(data)
print(f"wrote {len(FIXTURES)} fixtures to {os.path.normpath(OUT)}")
