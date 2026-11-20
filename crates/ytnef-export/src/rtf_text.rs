//! RTF to iCalendar text, as C `ytnef`'s `PrintRTF` and `ConsumeHyperlink`
//! do it: no RTF parser, just a scan that drops control words and groups
//! and escapes the rest for a `DESCRIPTION:` value.

use crate::print::cstr;

fn find(s: &[u8], from: usize, c: u8) -> Option<usize> {
    s[from..].iter().position(|&x| x == c).map(|i| from + i)
}

fn find_str(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    s[from..]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|i| from + i)
}

/// Append the text of decompressed RTF `rtf` to `out`, starting at its
/// first `\pard`, followed by a newline. Nothing at all (not even the
/// newline) when there is no `\pard`, as in C.
///
/// One divergence: C drops every `H` that does not start `HYPERLINK`
/// ("Hello" prints as "ello"); here it is kept.
pub fn rtf_text(out: &mut Vec<u8>, rtf: &[u8]) {
    // C scans the buffer as a C string
    let s = cstr(rtf);
    let Some(mut i) = find_str(s, 0, b"\\pard") else {
        return;
    };
    let mut slash = false;
    while i < s.len() {
        let c = s[i];
        match c {
            b'\\' => {
                slash = true;
                i += 1;
                continue;
            }
            b' ' if slash => {
                slash = false;
                i += 1;
                continue;
            }
            b'{' | b'}' => {
                slash = false;
                i += 1;
                continue;
            }
            b'l' if slash => {
                if s[i..].starts_with(b"listtext") {
                    out.extend_from_slice(b"*\t");
                    i += 1;
                    continue;
                }
                if s[i..].starts_with(b"lsdlocked") {
                    i += 9;
                    if i >= s.len() {
                        break;
                    }
                    if s[i].is_ascii_digit() {
                        match find(s, i, b';') {
                            Some(j) => i = j + 1,
                            None => break,
                        }
                        continue;
                    }
                    // C goes on with the byte after "lsdlocked" still in
                    // a control word, which skips it
                }
            }
            b'd' if slash && s[i..].starts_with(b"datastore") => {
                match find(s, i, b'}') {
                    Some(j) => i = j + 1,
                    None => break,
                }
                continue;
            }
            b'~' if slash => {
                out.push(b' ');
                slash = false;
                i += 1;
                continue;
            }
            _ => {}
        }
        if !slash {
            match c {
                b'H' if s[i..].starts_with(b"HYPERLINK") => {
                    i = hyperlink(out, s, i) + 1;
                    continue;
                }
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => {}
                b';' => out.extend_from_slice(b"\\;"),
                b',' => out.extend_from_slice(b"\\,"),
                _ => out.push(c),
            }
        }
        i += 1;
    }
    out.push(b'\n');
}

/// C `ConsumeHyperlink`: `s[at..]` starts with `HYPERLINK`. Print the
/// quoted link, as `text <link>` when the field result differs from it, and
/// return the index of the last byte consumed.
fn hyperlink(out: &mut Vec<u8>, s: &[u8], at: usize) -> usize {
    let Some((start, mut end)) =
        find(s, at, b'"').and_then(|q| Some((q + 1, find(s, q + 1, b'"')?)))
    else {
        // not a well-formed field: print it as plain text
        out.push(s[at]);
        return at;
    };
    let link = &s[start..end];
    let mut text = None;
    if let Some(f) = find_str(s, end, b"\\fldrslt") {
        let mut j = f;
        let mut slash = true;
        loop {
            j += 1;
            match s.get(j) {
                None | Some(b'}') => break,
                Some(b'\\') => slash = true,
                Some(b' ' | b'{') => slash = false,
                Some(_) if !slash => break,
                Some(_) => {}
            }
        }
        if s.get(j) != Some(&b'}') {
            let e = find(s, j, b'}').unwrap_or(s.len());
            text = Some(&s[j..e]);
            // C never leaves the caller on the terminating NUL
            end = if e == s.len() { e - 1 } else { e };
        }
    }
    match text {
        Some(t) if t != link => {
            out.extend_from_slice(t);
            out.extend_from_slice(b" <");
            out.extend_from_slice(link);
            out.push(b'>');
        }
        _ => out.extend_from_slice(link),
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rtf: &str) -> String {
        let mut v = Vec::new();
        rtf_text(&mut v, rtf.as_bytes());
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn needs_pard() {
        assert_eq!(text("{\\rtf1 plain}"), "");
    }

    #[test]
    fn drops_control_words_and_escapes() {
        assert_eq!(
            text("{\\rtf1{\\fonttbl x}\\pard\\f0 a, b; c\r\nd\\~e}"),
            "a\\, b\\; c\\nd e\n"
        );
    }

    #[test]
    fn lists_and_skipped_groups() {
        assert_eq!(
            text("\\pard{\\listtext 1.}x{\\*\\datastore 0102}y\\lsdlocked0 z;w"),
            "*\t1.x\n"
        );
        // the skipped group and number leave C inside a control word, so
        // the byte after each is dropped too
        assert_eq!(text("\\pard{\\*\\datastore 01} y"), "y\n");
        // a non-digit after lsdlocked is swallowed with the control word
        assert_eq!(text("\\pard\\lsdlockedx q"), "q\n");
        assert_eq!(text("\\pard\\lsdlocked"), "\n");
        assert_eq!(text("\\pard\\datastore 01"), "\n");
    }

    #[test]
    fn hyperlinks() {
        let field = |inner: &str| {
            text(&format!(
                "\\pard{{\\field{{\\*\\fldinst{{HYPERLINK \"http://x.test/\"}}}}{inner}}} after"
            ))
        };
        assert_eq!(
            field("{\\fldrslt{\\ul\\cf1 site}}"),
            "site <http://x.test/> after\n"
        );
        assert_eq!(
            field("{\\fldrslt{\\ul http://x.test/}}"),
            "http://x.test/ after\n"
        );
        // no field result: just the link
        assert_eq!(field(""), "http://x.test/ after\n");
        // unterminated result runs to the end
        assert_eq!(text("\\pard HYPERLINK \"l\" \\fldrslt abc"), "abc <l>\n");
        assert_eq!(text("\\pard HYPERLINK \"l\" \\fldrslt"), " <l>\n");
        // no quotes: printed as text
        assert_eq!(text("\\pard HYPERLINK x"), "HYPERLINK x\n");
    }

    #[test]
    fn keeps_capital_h() {
        // C prints "ello"
        assert_eq!(text("\\pard Hello"), "Hello\n");
    }

    #[test]
    fn stops_at_nul() {
        assert_eq!(text("\\pard a\0b"), "a\n");
    }
}
