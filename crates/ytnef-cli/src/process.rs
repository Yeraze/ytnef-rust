//! C `ProcessTNEF` for the `ytnef` binary: save bodies, exports and
//! attachments of a parsed stream. See the binary docs for how it differs
//! from C.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use crate::parse_reported;
use ytnef_core::mapi::{prop_tag, pt};
use ytnef_core::{has_signature, rtf, Limits, Tnef};
use ytnef_export::print::{rtf_error, subject};
use ytnef_export::{vcal, vcard};

const PR_CONVERSATION_TOPIC: u16 = 0x0070;
const PR_RTF_COMPRESSED: u16 = 0x1009;
const PR_BODY_HTML: u16 = 0x1013;
const PR_ATTACH_DATA_OBJ: u16 = 0x3701;
/// Attachment name properties, in the order C tries them:
/// `PR_ATTACH_LONG_FILENAME`, `PR_DISPLAY_NAME`, `PR_ATTACH_MIME_TAG`.
const NAME_PROPS: [u16; 3] = [0x3707, 0x3001, 0x370C];
/// C `MAX_FILENAME_SIZE` buffers hold this many bytes.
const C_NAME_MAX: usize = 1023;
/// Longest single path component most filesystems accept.
const NAME_MAX: usize = 255;

#[cfg(unix)]
pub fn os_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(s.as_bytes())
}

#[cfg(not(unix))]
pub fn os_bytes(s: &OsStr) -> Cow<'_, [u8]> {
    Cow::Owned(s.to_string_lossy().into_owned().into_bytes())
}

#[cfg(unix)]
fn bytes_os(b: &[u8]) -> Cow<'_, OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(OsStr::from_bytes(b))
}

#[cfg(not(unix))]
fn bytes_os(b: &[u8]) -> Cow<'_, OsStr> {
    Cow::Owned(OsString::from(String::from_utf8_lossy(b).into_owned()))
}

/// Bytes up to the first NUL, as C's `%s` reads them.
fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// C `SanitizeFilename` (message bodies): anything but ASCII alphanumerics
/// and `.` becomes `_`.
fn sanitize_body_name(name: &[u8]) -> Vec<u8> {
    name.iter()
        .map(|&c| {
            if c.is_ascii_alphanumeric() || c == b'.' {
                c
            } else {
                b'_'
            }
        })
        .collect()
}

/// C `SanitizeAttachmentName`'s replacements: space, `/` and `\` become `_`.
fn sanitize_attachment_name(name: &[u8]) -> Vec<u8> {
    name.iter()
        .map(|&c| {
            if matches!(c, b' ' | b'/' | b'\\') {
                b'_'
            } else {
                c
            }
        })
        .collect()
}

/// Stricter than C on top of the C sanitisers: no control bytes or
/// separators, no leading dots (so no `.`, `..` or hidden files), at most
/// [`NAME_MAX`] bytes. `None` when nothing usable is left.
fn safe_name(name: &[u8]) -> Option<Vec<u8>> {
    let start = name.iter().position(|&c| c != b'.')?;
    let mut v: Vec<u8> = name[start..]
        .iter()
        .map(|&c| {
            if c < 0x20 || c == 0x7f || c == b'/' || c == b'\\' {
                b'_'
            } else {
                c
            }
        })
        .collect();
    v.truncate(NAME_MAX);
    Some(v)
}

/// `name` with `n` inserted before its extension, cut to [`NAME_MAX`] bytes
/// by shortening the part before the extension.
fn numbered(name: &[u8], n: u32) -> Vec<u8> {
    let (stem, ext) = match name.iter().rposition(|&c| c == b'.') {
        Some(i) if i > 0 && name.len() - i <= 16 => name.split_at(i),
        _ => (name, &[][..]),
    };
    let num = if n == 0 { String::new() } else { n.to_string() };
    let keep = NAME_MAX.saturating_sub(num.len() + ext.len());
    let mut v = stem[..stem.len().min(keep)].to_vec();
    v.extend_from_slice(num.as_bytes());
    v.extend_from_slice(ext);
    v
}

/// `ytnef` flags that affect processing.
pub struct Opts {
    /// `-v` count; `+v` lowers it.
    pub verbose: i64,
    /// `-f`: write files to [`Opts::dir`].
    pub save_files: bool,
    /// `-F`: save the RTF body.
    pub save_rtf: bool,
    /// `-H`: save the HTML body.
    pub save_html: bool,
    /// `-a`: also save embedded TNEF streams.
    pub save_intermediate: bool,
    pub dir: Option<OsString>,
    pub limits: Limits,
}

impl Opts {
    pub fn verbose(&self) -> u32 {
        u32::try_from(self.verbose).unwrap_or(if self.verbose < 0 { 0 } else { u32::MAX })
    }

    /// The path C prints for `name`: `<dir>/<name>`, or `name` without `-f`.
    fn shown(&self, name: &[u8]) -> Vec<u8> {
        match &self.dir {
            Some(d) => [&os_bytes(d)[..], b"/", name].concat(),
            None => name.to_vec(),
        }
    }

    /// Create a new file for `name` in the output directory, never
    /// replacing an existing file or following a symlink. On a clash, try
    /// `name` with a counter. Returns the file and the name used.
    fn create(&self, name: &[u8]) -> io::Result<(File, Vec<u8>)> {
        let dir = Path::new(self.dir.as_deref().unwrap_or(OsStr::new(".")));
        for n in 0..1000 {
            let name = numbered(name, n);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(bytes_os(&name)))
            {
                Ok(f) => return Ok((f, name)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(ErrorKind::AlreadyExists.into())
    }

    /// Write `data` to a new file for `name` and print its path; on failure
    /// print C's error to stderr. Returns the failure count.
    fn save(&self, out: &mut impl Write, name: &[u8], data: &[u8]) -> io::Result<u32> {
        match self.create(name).and_then(|(mut f, name)| {
            f.write_all(data)?;
            Ok(name)
        }) {
            Ok(name) => {
                out.write_all(&self.shown(&name))?;
                out.write_all(b"\n")?;
                Ok(0)
            }
            Err(_) => {
                out.flush()?;
                let shown = self.shown(name);
                eprintln!(
                    "ERROR: Error writing {} to disk!",
                    String::from_utf8_lossy(&shown)
                );
                Ok(1)
            }
        }
    }

    /// Decompressed `PR_RTF_COMPRESSED`, printing C's message if it fails.
    fn rtf_body(&self, out: &mut impl Write, t: &Tnef) -> io::Result<Option<Vec<u8>>> {
        let Some(v) = first(t, pt::BINARY, PR_RTF_COMPRESSED) else {
            return Ok(None);
        };
        match rtf::decompress_rtf(v, &self.limits) {
            Ok(r) => Ok(Some(r)),
            Err(e) => {
                out.write_all(rtf_error(&e).as_bytes())?;
                Ok(None)
            }
        }
    }

    /// C `ProcessTNEF`: save the bodies and attachments of `t`, recursing
    /// into embedded TNEF streams. Returns the failure count.
    pub fn process(&self, out: &mut impl Write, t: &Tnef, depth: u8) -> io::Result<u32> {
        let mut failures = 0;
        let class = cstr(t.message_class.unwrap_or_default());
        let class = &class[..class.len().min(49)];
        let has_subject = t.subject.is_some_and(|s| !s.is_empty());

        if class == b"IPM.Contact" {
            let name = body_name(&vcard::vcard_name(t), b".vcard");
            failures += self.export(out, &name, || vcard::vcard(t))?;
        }
        if class == b"IPM.Task" {
            if let Some(base) = vcal::vtask_name(t) {
                let name = body_name(&base, b".vcf");
                failures += self.export(out, &name, || vcal::vtask(t))?;
            }
        }
        let appointment = t
            .mapi
            .get_named(prop_tag(pt::STRING8, 0x24))
            .and_then(|p| p.values.first())
            .is_some_and(|v| cstr(v) == b"IPM.Appointment");
        let mtg_req = class == b"IPM.Microsoft Schedule.MtgReq";
        if mtg_req || class == b"IPM.Appointment" || appointment {
            let name: &[u8] = if mtg_req {
                b"MtgReq.ics"
            } else {
                b"calendar.ics"
            };
            failures += self.calendar(out, t, class, name)?;
        }

        if class == b"IPM.Microsoft Mail.Note" && has_subject {
            let subj = subject(t).unwrap_or_default();
            if self.save_rtf {
                if let Some(body) = self.rtf_body(out, t)? {
                    failures += self.save_body(out, &subj, b".rtf", &body)?;
                }
            }
            if self.save_html {
                if let Some(body) = first(t, pt::BINARY, PR_BODY_HTML) {
                    failures += self.save_body(out, &subj, b".html", body)?;
                }
            }
        }

        if class == b"IPM.StickyNote" && self.save_rtf && has_subject {
            let topic = first(t, pt::STRING8, PR_CONVERSATION_TOPIC)
                .or_else(|| first(t, pt::UNICODE, PR_CONVERSATION_TOPIC));
            if let Some(body) = self.rtf_body(out, t)? {
                if let Some(topic) = topic {
                    // C copies at most 31 bytes, up to a NUL (so one
                    // character of a UTF-16 topic)
                    let base = cstr(topic);
                    let base = sanitize_body_name(&base[..base.len().min(31)]);
                    failures += self.save_body(out, &base, b".rtf", &body)?;
                }
            }
        }

        for (i, a) in t.attachments.iter().enumerate() {
            let Some(data) = a.data.filter(|d| !d.is_empty()) else {
                continue;
            };
            let object = first_of(&a.mapi, pt::OBJECT, PR_ATTACH_DATA_OBJ)
                .or_else(|| first_of(&a.mapi, pt::BINARY, PR_ATTACH_DATA_OBJ));
            // an attached object starts with a 16-byte identifier
            let payload = match object {
                Some(o) => o.get(16..).unwrap_or_default(),
                None => data,
            };

            let mut real = true;
            if has_signature(payload) && depth < self.limits.max_nesting {
                if self.verbose >= 1 {
                    writeln!(
                        out,
                        "DEBUG(1/{}): Attempting to parse memory block...\n",
                        self.verbose
                    )?;
                }
                if let Ok(inner) = parse_reported(out, payload, self.verbose(), &self.limits)? {
                    failures += self.process(out, &inner, depth + 1)?;
                    real = false;
                }
            }

            if (real || self.save_intermediate) && self.save_files {
                let name = NAME_PROPS
                    .iter()
                    .find_map(|&id| first_of(&a.mapi, pt::STRING8, id))
                    .or(a.title)
                    .unwrap_or_default();
                let name = cstr(name);
                let name = sanitize_attachment_name(&name[..name.len().min(C_NAME_MAX)]);
                let name = safe_name(&name)
                    .unwrap_or_else(|| format!("file_{:03}.dat", i + 1).into_bytes());
                failures += self.save(out, &name, payload)?;
            }
        }
        Ok(failures)
    }

    /// Save a message body as `<base><ext>`, named by [`body_name`].
    fn save_body(
        &self,
        out: &mut impl Write,
        base: &[u8],
        ext: &[u8],
        body: &[u8],
    ) -> io::Result<u32> {
        self.save(out, &body_name(base, ext), body)
    }

    /// C `SaveVCard` / `SaveVTask`: print the file name, and with `-f`
    /// write the file. Returns the failure count.
    fn export(
        &self,
        out: &mut impl Write,
        name: &[u8],
        data: impl FnOnce() -> Vec<u8>,
    ) -> io::Result<u32> {
        if self.save_files {
            return self.save(out, name, &data());
        }
        out.write_all(name)?;
        out.write_all(b"\n")?;
        Ok(0)
    }

    /// C `SaveVCalendar`: like [`Opts::export`], but the name gets a counter
    /// when the file exists (also without `-f`, where C checks the current
    /// directory), and an RTF description that fails to decompress prints
    /// its error after the name.
    fn calendar(
        &self,
        out: &mut impl Write,
        t: &Tnef,
        class: &[u8],
        name: &[u8],
    ) -> io::Result<u32> {
        if !self.save_files {
            let name = (0..1000)
                .map(|n| numbered(name, n))
                .find(|n| !Path::new(&*bytes_os(n)).exists())
                .unwrap_or_else(|| name.to_vec());
            out.write_all(&name)?;
            out.write_all(b"\n")?;
            return Ok(0);
        }
        let (data, err) = vcal::vcalendar(t, class, &self.limits);
        let failures = self.save(out, name, &data)?;
        if let Some(e) = err {
            out.write_all(rtf_error(&e).as_bytes())?;
        }
        Ok(failures)
    }

    /// Parse and process one file. Returns the failure count.
    pub fn file(&self, out: &mut impl Write, path: &OsStr) -> io::Result<u32> {
        if self.verbose >= 1 {
            out.write_all(b"Attempting to parse ")?;
            out.write_all(&os_bytes(path))?;
            out.write_all(b"...\n")?;
        }
        let parsed = match std::fs::read(path) {
            Ok(buf) => match parse_reported(out, &buf, self.verbose(), &self.limits)? {
                Ok(t) => Some(self.process(out, &t, 0)?),
                Err(_) => None,
            },
            Err(_) => None,
        };
        Ok(parsed.unwrap_or_else(|| {
            let _ = out.flush();
            eprintln!("ERROR processing file");
            1
        }))
    }
}

/// The file name C gives a body or export: `<base><ext>` after
/// `SanitizeFilename`, cut to [`NAME_MAX`] bytes by shortening `base`. An
/// empty base becomes `body` (C writes a hidden `<ext>` file).
fn body_name(base: &[u8], ext: &[u8]) -> Vec<u8> {
    let base = sanitize_body_name(&base[..base.len().min(C_NAME_MAX - ext.len())]);
    let mut base = safe_name(&base).unwrap_or_else(|| b"body".to_vec());
    base.truncate(NAME_MAX - ext.len());
    [&base[..], ext].concat()
}

/// First value of the non-named property `PROP_TAG(ty, id)` in `t`'s message
/// properties.
fn first<'a>(t: &Tnef<'a>, ty: u16, id: u16) -> Option<&'a [u8]> {
    first_of(&t.mapi, ty, id)
}

fn first_of<'a>(props: &ytnef_core::mapi::MapiProps<'a>, ty: u16, id: u16) -> Option<&'a [u8]> {
    props
        .get(prop_tag(ty, id))
        .and_then(|p| p.values.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_is_one_component() {
        let c = |n: &[u8]| safe_name(&sanitize_attachment_name(n));
        // C keeps "../x" as ".._x"; leading dots are stripped here
        assert_eq!(c(b"../x"), Some(b"_x".to_vec()));
        assert_eq!(c(b"..\\..\\x"), Some(b"_.._x".to_vec()));
        assert_eq!(c(b"a\x1bb\nc"), Some(b"a_b_c".to_vec()));
        assert_eq!(c(b".bashrc"), Some(b"bashrc".to_vec()));
        assert_eq!(c(b".."), None);
        assert_eq!(c(b""), None);
        assert_eq!(c(&[b'a'; 300]).map(|v| v.len()), Some(NAME_MAX));
    }

    #[test]
    fn numbered_keeps_extension() {
        assert_eq!(numbered(b"x.dat", 0), b"x.dat");
        assert_eq!(numbered(b"x.dat", 2), b"x2.dat");
        assert_eq!(numbered(b"noext", 1), b"noext1");
        assert_eq!(numbered(b".rc", 1), b".rc1");
        let long = [&[b'a'; 300][..], b".txt"].concat();
        let n = numbered(&long, 12);
        assert_eq!(n.len(), NAME_MAX);
        assert!(n.ends_with(b"12.txt"));
    }

    #[test]
    fn body_names_follow_c() {
        assert_eq!(sanitize_body_name(b"Re: hi/there.rtf"), b"Re__hi_there.rtf");
    }
}
