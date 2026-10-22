//! Bounded display decoding. Sources and physical coordinates remain byte based.
use serde_json::{json, Value};
use std::borrow::Cow;

/// Whether `head`, the start of a file that failed on NUL, is UTF-16 text
/// rather than some other binary: it starts with a byte order mark (UTF-32's
/// marks included), or one byte of nearly every pair is NUL while the other
/// seldom is, as mostly ASCII text (log stamps, IDs and tags) is in UTF-16.
/// Unmarked text that is mostly non-ASCII, and unmarked UTF-32, keep the
/// general NUL error. Logs that Windows PowerShell saves with `>` are
/// UTF-16LE with a mark.
pub fn wide_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE])
        || head.starts_with(&[0xFE, 0xFF])
        || head.starts_with(&[0, 0, 0xFE, 0xFF])
    {
        return true;
    }
    let pairs = head.len() / 2;
    if pairs < 8 {
        return false;
    }
    let nul = |first: usize| {
        head[..pairs * 2]
            .iter()
            .skip(first)
            .step_by(2)
            .filter(|&&b| b == 0)
            .count()
    };
    let (even, odd) = (nul(0), nul(1));
    // 70 % allows for non-ASCII text, whose high byte is not NUL. The low
    // byte is NUL for one code point in 256, among them common Hangul
    // syllables such as 글 (U+AE00), so it is allowed in a tenth.
    let wide = |nul: usize, other: usize| nul * 10 >= pairs * 7 && other * 10 <= pairs;
    wide(odd, even) || wide(even, odd)
}
/// Whether `head` starts a compressed archive: gzip, zip, bzip2, xz, zstd,
/// 7z or RAR, by their magic bytes. Their headers hold NUL within the first
/// bytes, so such a file fails as a text log; this names the reason.
pub fn archive(head: &[u8]) -> bool {
    const MAGICS: [&[u8]; 9] = [
        b"\x1f\x8b",
        b"PK\x03\x04",
        b"PK\x05\x06",
        b"PK\x07\x08",
        b"BZh",
        b"\xfd7zXZ\x00",
        b"\x28\xb5\x2f\xfd",
        b"7z\xbc\xaf\x27\x1c",
        b"Rar!\x1a\x07",
    ];
    MAGICS.iter().any(|m| head.starts_with(m))
}
pub fn decode(data: &[u8]) -> Cow<'_, str> {
    // `from_utf8` skips ASCII words at a time; the lossy scan goes byte by
    // byte, so it only runs on damaged text.
    match std::str::from_utf8(data) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => String::from_utf8_lossy(data),
    }
}

struct Part<'a> {
    start: usize,
    end: usize,
    text: &'a str,
    damaged: bool,
}
struct Parts<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Parts<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
}
impl<'a> Iterator for Parts<'a> {
    type Item = Part<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.at == self.data.len() {
            return None;
        }
        let start = self.at;
        let (len, damaged) = match std::str::from_utf8(&self.data[start..]) {
            Ok(_) => (self.data.len() - start, false),
            Err(e) if e.valid_up_to() > 0 => (e.valid_up_to(), false),
            Err(e) => (e.error_len().unwrap_or(self.data.len() - start), true),
        };
        self.at += len;
        Some(Part {
            start,
            end: self.at,
            text: if damaged {
                "\u{fffd}"
            } else {
                std::str::from_utf8(&self.data[start..self.at]).unwrap()
            },
            damaged,
        })
    }
}

/// Align an original byte offset and choose a display window <= max UTF-8 bytes.
/// Walk original units, so slicing cannot manufacture a new damaged sequence.
pub fn window(data: &[u8], start: usize, max: usize) -> (usize, usize) {
    if start == data.len() {
        return (start, start);
    }
    let (mut from, mut end, mut used, mut begun) = (start, start, 0, false);
    for p in Parts::new(data) {
        if !begun {
            if start >= p.end {
                continue;
            }
            from = if p.damaged {
                p.start
            } else {
                let mut at = start - p.start;
                while !p.text.is_char_boundary(at) {
                    at -= 1;
                }
                p.start + at
            };
            end = from;
            begun = true;
        }
        if p.damaged {
            if used + 3 > max {
                break;
            }
            used += 3;
            end = p.end;
        } else {
            let at = end.max(p.start) - p.start;
            let remaining = &p.text[at..];
            let mut take = remaining.len().min(max - used);
            while !remaining.is_char_boundary(take) {
                take -= 1;
            }
            used += take;
            end = p.start + at + take;
            if take < remaining.len() {
                break;
            }
        }
        if used == max {
            break;
        }
    }
    (from, end)
}

/// Display byte offsets of generated replacements, ascending. One pass lets a
/// caller skip `damage` for fields that contain none.
pub fn replacement_offsets(data: &[u8]) -> Vec<usize> {
    let (mut display, mut out) = (0, Vec::new());
    for p in Parts::new(data) {
        if p.damaged {
            out.push(display);
        }
        display += p.text.len();
    }
    out
}

/// One bit per visible UTF-16 position; only generated replacements set bits.
/// No all-file damage list. Hex samples are capped at 16 entries/16 bytes each.
/// display_start/end are byte offsets within the decoded row, not source bytes.
pub fn damage(data: &[u8], display_start: usize, display_end: usize, source_base: u64) -> Value {
    let mut mask = vec![0u8; (display_end - display_start).div_ceil(8)];
    let (mut display, mut units, mut damaged_bytes, mut count) = (0usize, 0usize, 0usize, 0usize);
    let mut samples = Vec::new();
    for p in Parts::new(data) {
        let next = display + p.text.len();
        if display >= display_end {
            break;
        }
        if next > display_start {
            if p.damaged {
                mask[units / 8] |= 1 << (units % 8);
                count += 1;
                damaged_bytes += p.end - p.start;
                if samples.len() < 16 {
                    samples.push(json!({"position":units,"startByte":p.start,"endByte":p.end,"sourceStartByte":source_base+p.start as u64,"hex":data[p.start..p.end.min(p.start+16)].iter().map(|b|format!("{b:02X}")).collect::<Vec<_>>().join(" ")}));
                }
                units += 1;
            } else {
                let lo = display_start.saturating_sub(display);
                let hi = p.text.len().min(display_end - display);
                units += p.text[lo..hi].encode_utf16().count();
            }
        }
        display = next;
    }
    while mask.last() == Some(&0) {
        mask.pop();
    }
    json!({"mask":mask.iter().map(|b|format!("{b:02x}")).collect::<String>(),"bytes":damaged_bytes,"replacements":count,"samples":samples,"samplesTruncated":count>16})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_distinguish_actual_replacement_and_source_bytes() {
        let b = b"\xef\xbf\xbd\xff\xe2\x82\xed\xa0\x80";
        assert_eq!(decode(b), "������");
        let m = damage(b, 0, decode(b).len(), 40);
        assert_eq!(m["mask"], "3e");
        assert_eq!(m["bytes"], 6);
        assert_eq!(m["samples"][0]["sourceStartByte"], 43);
    }
    #[test]
    fn wide_text_names_utf16_and_not_other_binaries() {
        let row = "09-21 12:07:30.386  1234  5678 I 한글Tag: message 🙂\r\n";
        let le: Vec<u8> = row.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = row.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert!(wide_text(&le) && wide_text(&be));
        assert!(wide_text(&[&[0xFF, 0xFE], &le[..]].concat()));
        assert!(wide_text(&[&[0xFE, 0xFF], &be[..]].concat()));
        assert!(wide_text(b"\xff\xfe"), "a mark alone");
        assert!(
            wide_text(&[0, 0, 0xFE, 0xFF, 0, 0, 0, b'a']),
            "UTF-32BE mark"
        );
        // Korean-heavy UTF-16 still has NUL high bytes in its stamps and IDs.
        let korean: Vec<u8> = "09-21 12:07:30.386 1 2 I 태그: 한글 메시지 한글 메시지\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(wide_text(&korean));
        // UTF-8 with a NUL, all-zero data, random-looking binary and short
        // heads are not UTF-16.
        assert!(!wide_text(b"valid\0\xff text with one NUL in it\n"));
        assert!(!wide_text(&[0; 64]));
        assert!(!wide_text(&[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
        ]));
        let mut binary = vec![];
        for i in 0..200u32 {
            binary.extend_from_slice(&(i.wrapping_mul(2_654_435_761)).to_le_bytes());
        }
        assert!(!wide_text(&binary));
        assert!(!wide_text(&le[..14]), "under eight pairs");
    }
    #[test]
    fn archives_are_named_by_their_magic_bytes() {
        for head in [
            &b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03"[..],
            b"PK\x03\x04\x14\x00\x00\x00\x08\x00",
            b"BZh91AY&SY",
            b"\xfd7zXZ\x00\x00\x04",
            b"\x28\xb5\x2f\xfd\x04\x00",
            b"7z\xbc\xaf\x27\x1c\x00\x04",
            b"Rar!\x1a\x07\x01\x00",
        ] {
            assert!(archive(head), "{head:?}");
        }
        for head in [
            &b""[..],
            b"\x1f",
            b"PK\x01\x02",
            b"09-21 12:07:30.386",
            b"DLT\x01",
        ] {
            assert!(!archive(head), "{head:?}");
        }
    }
    #[test]
    fn detail_windows_keep_original_offsets_and_complete_units() {
        let b = ["한🙂".as_bytes(), b"\xff\xe2\x82"].concat();
        for start in 0..=b.len() {
            let (from, to) = window(&b, start, 6);
            assert!(from <= start && to >= from);
            assert!(decode(&b[from..to]).len() <= 6);
        }
        assert_eq!(window(&b, 4, 4), (3, 7));
        assert_eq!(window(&b, 9, 3), (8, 10));
    }
}
