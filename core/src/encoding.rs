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
/// Display text of source bytes: invalid UTF-8 becomes U+FFFD.
pub fn decode(data: &[u8]) -> Cow<'_, str> {
    decoded(data, false).text
}
/// `decode` with what it changed.
pub struct Decoded<'a> {
    pub text: Cow<'a, str>,
    /// Some bytes were invalid UTF-8 and show as U+FFFD.
    pub damaged: bool,
    /// Some text was repaired from Mac OS Roman mojibake.
    pub repaired: bool,
}
/// `decode`, which also repairs Mac OS Roman mojibake when `repair` is set:
/// for the rows of an iOS log, whose tools garble that way (see `repair`).
pub fn decoded(data: &[u8], repair: bool) -> Decoded<'_> {
    // `from_utf8` skips ASCII words at a time; the lossy scan goes byte by
    // byte, so it only runs on damaged text.
    match std::str::from_utf8(data) {
        Ok(text) => {
            let text = shown(text, repair);
            let repaired = matches!(text, Cow::Owned(_));
            Decoded {
                text,
                damaged: false,
                repaired,
            }
        }
        Err(_) => {
            let lossy = String::from_utf8_lossy(data).into_owned();
            let fixed = match shown(&lossy, repair) {
                Cow::Owned(fixed) => Some(fixed),
                Cow::Borrowed(_) => None,
            };
            Decoded {
                repaired: fixed.is_some(),
                text: Cow::Owned(fixed.unwrap_or(lossy)),
                damaged: true,
            }
        }
    }
}

/// Mac OS Roman's upper half, bytes 0x80 to 0xFF, as Apple maps it.
const MAC_ROMAN: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', 'ê', 'ë', 'í',
    'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', '†', '°', '¢', '£', '§', '•',
    '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏',
    'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{a0}',
    'À', 'Ã', 'Õ', 'Œ', 'œ', '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›',
    'ﬁ', 'ﬂ', '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô',
    '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];
/// Mac OS Roman bytes of U+00A0 to U+00FF; 0 where it has none.
const MAC_ROMAN_LATIN1: [u8; 96] = {
    let mut table = [0; 96];
    let mut i = 0;
    while i < 128 {
        let c = MAC_ROMAN[i] as usize;
        if c >= 0xA0 && c <= 0xFF {
            table[c - 0xA0] = 0x80 + i as u8;
        }
        i += 1;
    }
    table
};
fn mac_roman(c: char) -> Option<u8> {
    match c as usize {
        0..=0x9F => None,
        n @ 0xA0..=0xFF => Some(MAC_ROMAN_LATIN1[n - 0xA0]).filter(|&b| b != 0),
        // The rest of `MAC_ROMAN`; a test checks they agree.
        0x0131 => Some(0xF5),
        0x0152 => Some(0xCE),
        0x0153 => Some(0xCF),
        0x0178 => Some(0xD9),
        0x0192 => Some(0xC4),
        0x02C6 => Some(0xF6),
        0x02C7 => Some(0xFF),
        0x02D8 => Some(0xF9),
        0x02D9 => Some(0xFA),
        0x02DA => Some(0xFB),
        0x02DB => Some(0xFE),
        0x02DC => Some(0xF7),
        0x02DD => Some(0xFD),
        0x03A9 => Some(0xBD),
        0x03C0 => Some(0xB9),
        0x2013 => Some(0xD0),
        0x2014 => Some(0xD1),
        0x2018 => Some(0xD4),
        0x2019 => Some(0xD5),
        0x201A => Some(0xE2),
        0x201C => Some(0xD2),
        0x201D => Some(0xD3),
        0x201E => Some(0xE3),
        0x2020 => Some(0xA0),
        0x2021 => Some(0xE0),
        0x2022 => Some(0xA5),
        0x2026 => Some(0xC9),
        0x2030 => Some(0xE4),
        0x2039 => Some(0xDC),
        0x203A => Some(0xDD),
        0x2044 => Some(0xDA),
        0x20AC => Some(0xDB),
        0x2122 => Some(0xAA),
        0x2202 => Some(0xB6),
        0x2206 => Some(0xC6),
        0x220F => Some(0xB8),
        0x2211 => Some(0xB7),
        0x221A => Some(0xC3),
        0x221E => Some(0xB0),
        0x222B => Some(0xBA),
        0x2248 => Some(0xC5),
        0x2260 => Some(0xAD),
        0x2264 => Some(0xB2),
        0x2265 => Some(0xB3),
        0x25CA => Some(0xD7),
        0xF8FF => Some(0xF0),
        0xFB01 => Some(0xDE),
        0xFB02 => Some(0xDF),
        _ => None,
    }
}
/// Finds the characters whose UTF-8 starts with one of the bytes below. They
/// start every character that stands for a UTF-8 lead byte (0xC2 to 0xF4),
/// which starts a repaired run, and some others. All are lead bytes, so a
/// found byte starts a character.
struct Leads<'a> {
    s: &'a [u8],
    /// The next byte of each memchr set at or after where it was last
    /// searched from; `s.len()` for none.
    next: [usize; 2],
}
impl<'a> Leads<'a> {
    const SETS: [[u8; 3]; 2] = [[0xC2, 0xC3, 0xE2], [0xC5, 0xC6, 0xEF]];
    fn new(s: &'a [u8]) -> Self {
        let mut leads = Self { s, next: [0; 2] };
        for k in 0..2 {
            leads.next[k] = leads.search(k, 0);
        }
        leads
    }
    fn search(&self, k: usize, at: usize) -> usize {
        let [a, b, c] = Self::SETS[k];
        memchr::memchr3(a, b, c, &self.s[at..]).map_or(self.s.len(), |i| at + i)
    }
    /// The next one from `at`, a character boundary. Steps over ASCII eight
    /// bytes at a time and over other characters whole while they are near,
    /// where memchr restarts would cost more; after eight other characters
    /// it searches with memchr, which each set resumes from its last find.
    fn next(&mut self, mut at: usize) -> Option<usize> {
        let s = self.s;
        let mut others = 0;
        loop {
            while let Some(word) = s.get(at..at + 8) {
                if u64::from_ne_bytes(word.try_into().unwrap()) & 0x8080_8080_8080_8080 != 0 {
                    break;
                }
                at += 8;
            }
            at += match *s.get(at)? {
                0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xE2 | 0xEF => return Some(at),
                0..=0x7F => 1,
                _ if others == 8 => break,
                b => {
                    others += 1;
                    match b {
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        _ => 4,
                    }
                }
            };
        }
        for k in 0..2 {
            if self.next[k] < at {
                self.next[k] = self.search(k, at);
            }
        }
        Some(self.next[0].min(self.next[1])).filter(|&i| i < s.len())
    }
}
/// Whether repaired text reads as text rather than as two ordinary
/// characters that happen to form UTF-8. One two-byte character must be a
/// Latin letter or sign ("√©" as "é"): a curly quote or no-break space
/// before an accent, as in "l’école" or "20\u{a0}°C", would become a
/// Cyrillic, Armenian or IPA letter. Text may not start with a combining
/// mark, which Windows-1252 mojibake such as "Ã©" and quotes before capital
/// accents would become, nor hold C1 controls or the line and paragraph
/// separators that end parsed rows.
fn plausible(fixed: &str) -> bool {
    let mut chars = fixed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if fixed.len() == 2 && !('\u{a0}'..='\u{24f}').contains(&first) {
        return false;
    }
    let combining = matches!(first,
        '\u{300}'..='\u{36f}'
        | '\u{483}'..='\u{489}'
        | '\u{591}'..='\u{5bd}'
        | '\u{5bf}'
        | '\u{5c1}'..='\u{5c2}'
        | '\u{5c4}'..='\u{5c5}'
        | '\u{5c7}'
        | '\u{610}'..='\u{61a}'
        | '\u{64b}'..='\u{65f}'
        | '\u{670}'
        | '\u{6d6}'..='\u{6dc}'
        | '\u{6df}'..='\u{6e4}'
        | '\u{6e7}'..='\u{6e8}'
        | '\u{6ea}'..='\u{6ed}'
        | '\u{711}'
        | '\u{730}'..='\u{74a}'
        | '\u{7a6}'..='\u{7b0}'
        | '\u{7eb}'..='\u{7f3}'
        | '\u{1ab0}'..='\u{1aff}'
        | '\u{1dc0}'..='\u{1dff}'
        | '\u{20d0}'..='\u{20ff}'
        | '\u{fe20}'..='\u{fe2f}');
    !combining
        && !fixed
            .chars()
            .any(|c| matches!(c, '\u{80}'..='\u{9f}' | '\u{2028}' | '\u{2029}'))
}
/// Repairs UTF-8 that was read as Mac OS Roman and written as UTF-8 again,
/// as some iOS tools write non-ASCII text: "안녕" as "ÏïàÎÖï". Each such
/// character stands for one original byte; ASCII stays itself, so a run of
/// characters from Mac OS Roman's upper half is one original non-ASCII run.
///
/// A whole run is the unit, so ordinary accented text, whose characters
/// seldom adjoin, stays as it is. Its longest prefix that is valid UTF-8 is
/// repaired when `plausible`, which keeps a run cut short or followed by a
/// real symbol readable. A run that starts mid-character, as a detail window
/// may cut one, stays as it is. Line breaks are ASCII, so rows keep their
/// boundaries.
pub fn repair(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let s = text.as_bytes();
    // Most text has no such lead at all; memchr finds that fastest.
    let mut leads = Leads::new(s);
    if leads.next[0].min(leads.next[1]) == s.len() {
        return Cow::Borrowed(text);
    }
    let (mut out, mut copied, mut at, mut bytes) = (String::new(), 0, 0, vec![]);
    while let Some(i) = leads.next(at) {
        let len = if s[i] < 0xE0 { 2 } else { 3 };
        // The UTF-8 of every `MAC_ROMAN` character starts with one of these.
        let roman = |b: &u8| matches!(b, 0xC2..=0xC6 | 0xCB | 0xCE | 0xCF | 0xE2 | 0xEF);
        if !s.get(i + len).is_some_and(roman) {
            // No run of two from here: a lone symbol or accent before other
            // text, or a run that ends here, which started at no lead.
            at = i + len;
            continue;
        }
        // Runs are taken whole: one may start before its first lead.
        let start = text[at..i]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| mac_roman(c).is_some())
            .last()
            .map_or(i, |(k, _)| at + k);
        bytes.clear();
        let mut end = start;
        for c in text[start..].chars() {
            let Some(b) = mac_roman(c) else { break };
            bytes.push(b);
            end += c.len_utf8();
        }
        let valid = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(e) => e.valid_up_to(),
        };
        let fixed = std::str::from_utf8(&bytes[..valid]).unwrap();
        if plausible(fixed) {
            // Each original byte was one character.
            let to = text[start..]
                .char_indices()
                .nth(valid)
                .map_or(end, |(k, _)| start + k);
            out.push_str(&text[copied..start]);
            out.push_str(fixed);
            copied = to;
        }
        // Past the run, or past the lead when it starts none.
        at = end.max(i + len);
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
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

/// A part's display text, repaired as `decoded` repairs the whole.
fn shown(text: &str, repair: bool) -> Cow<'_, str> {
    if repair {
        self::repair(text)
    } else {
        Cow::Borrowed(text)
    }
}
/// Display byte offsets of generated replacements, ascending. One pass lets a
/// caller skip `damage` for fields that contain none.
pub fn replacement_offsets(data: &[u8], repair: bool) -> Vec<usize> {
    let (mut display, mut out) = (0, Vec::new());
    for p in Parts::new(data) {
        if p.damaged {
            out.push(display);
        }
        display += shown(p.text, repair).len();
    }
    out
}

/// One bit per visible UTF-16 position; only generated replacements set bits.
/// No all-file damage list. Hex samples are capped at 16 entries/16 bytes each.
/// display_start/end are byte offsets within the decoded row, not source bytes.
pub fn damage(
    data: &[u8],
    repair: bool,
    display_start: usize,
    display_end: usize,
    source_base: u64,
) -> Value {
    let mut mask = vec![0u8; (display_end - display_start).div_ceil(8)];
    let (mut display, mut units, mut damaged_bytes, mut count) = (0usize, 0usize, 0usize, 0usize);
    let mut samples = Vec::new();
    for p in Parts::new(data) {
        // Runs never span a replacement, so parts repair as the whole does.
        let text = shown(p.text, repair);
        let next = display + text.len();
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
                let hi = text.len().min(display_end - display);
                units += text[lo..hi].encode_utf16().count();
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
        let m = damage(b, false, 0, decode(b).len(), 40);
        assert_eq!(m["mask"], "3e");
        assert_eq!(m["bytes"], 6);
        assert_eq!(m["samples"][0]["sourceStartByte"], 43);
    }
    /// `text`'s UTF-8 read as Mac OS Roman, as the tools that garble it do.
    fn garble(text: &str) -> String {
        text.bytes()
            .map(|b| match b {
                0..=0x7F => b as char,
                _ => MAC_ROMAN[b as usize - 0x80],
            })
            .collect()
    }
    #[test]
    fn mac_roman_mojibake_is_repaired_by_whole_runs() {
        assert_eq!(garble("안녕"), "ÏïàÎÖï");
        let line = "Greeting(text : 안녕하세요, lang : ko, city : 대전,";
        assert_eq!(repair(&garble(line)), line);
        // Emoji start with Apple's logo; quotes and accents repair too.
        for text in [
            "🙂 ok",
            "“quoted” – dash",
            "café Grüße",
            "€ ✓ 日本語\r\n",
            "Привет мир Ωμέγα",
        ] {
            assert_eq!(repair(&garble(text)), text);
        }
        // One two-byte letter beyond Latin is too like ordinary text.
        assert_eq!(repair(&garble("Ω Ж")), garble("Ω Ж"));
        // A run cut short, or followed by a real symbol, keeps its rest.
        let cut = garble("대전");
        let cut = cut.strip_suffix('Ñ').unwrap();
        assert_eq!(repair(cut), "대Ï†");
        assert_eq!(repair(&format!("{}°C", garble("대전"))), "대전°C");
        // Text that is no Mac OS Roman mojibake stays borrowed.
        for text in [
            "plain ascii",
            "안녕 대전 🙂",
            "café naïve Grüße à la crème",
            "“quoted” – … ± 5 °C ≤ µs",
            // Windows-1252 mojibake would become combining marks or C1
            // controls, and that of Korean starts invalid.
            "cafÃ© Ã¨ Ã¶ Ã§",
            "í•œêµ\u{AD}",
            "¬Ä",
            // Quotes, spaces and signs before accents form UTF-8 by chance.
            "l’école d’été qu’à",
            "“école” ‘área’ “Éxito”",
            "€à Àé",
            "20\u{a0}°C 10\u{a0}µs",
            // A line separator would end a parsed row.
            "a‚Ä®b ‚Ä©",
        ] {
            assert!(matches!(repair(text), Cow::Borrowed(_)), "{text}");
        }
        let mut all = MAC_ROMAN.iter().collect::<String>();
        all.push_str("Ð←");
        let _ = repair(&all);
    }
    #[test]
    fn mac_roman_bytes_invert_the_table() {
        for (i, &c) in MAC_ROMAN.iter().enumerate() {
            assert_eq!(mac_roman(c), Some(0x80 + i as u8), "{c}");
        }
        let all = (0..=0x10FFFF).filter_map(char::from_u32);
        assert_eq!(all.filter_map(mac_roman).count(), 128);
    }
    #[test]
    fn random_text_round_trips_and_keeps_rows() {
        let pieces = [
            "a", " ", "\n", "\r\n", "[", "안", "녕", "🙂", "é", "ü", "“", "–", "…", "°", "Ã", "©",
            "√", "\u{f8ff}", "ÿ", "Ω", "π", "\u{301}", "\u{a0}", "日", "Ж",
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..seed % 24 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                text.push_str(pieces[(seed % pieces.len() as u64) as usize]);
            }
            let fixed = repair(&text);
            let breaks = |t: &str| t.match_indices(['\r', '\n']).count();
            assert_eq!(breaks(&fixed), breaks(&text), "{text:?}");
            assert!(fixed.len() <= text.len());
            // Garbled text reads as it was, unless it has combining marks or
            // two-byte letters beyond Latin, which repair only in words.
            if !text.contains(|c| matches!(c, '\u{301}' | '\u{250}'..='\u{7ff}')) {
                assert_eq!(repair(&garble(&text)), text, "{text:?}");
            }
        }
    }
    #[test]
    fn repaired_rows_keep_damage_marks_on_their_display() {
        let b = [
            garble("안녕").as_bytes(),
            b"\xff",
            garble("녕").as_bytes(),
            b" x",
        ]
        .concat();
        let d = decoded(&b, true);
        assert_eq!(d.text, "안녕\u{fffd}녕 x");
        assert!(d.damaged && d.repaired);
        assert_eq!(replacement_offsets(&b, true), vec![6]);
        assert_eq!(damage(&b, true, 0, d.text.len(), 0)["mask"], "04");
        // A field after the replacement counts from its own start.
        assert_eq!(damage(&b, true, 6, d.text.len(), 0)["mask"], "01");
        // Without repair the garbled text and its offsets stay.
        let plain = decoded(&b, false);
        assert!(plain.damaged && !plain.repaired);
        assert_eq!(plain.text, decode(&b));
        assert_eq!(replacement_offsets(&b, false), vec![garble("안녕").len()]);
        let garbled = garble("안녕");
        let clean = decoded(garbled.as_bytes(), true);
        assert!(clean.repaired && !clean.damaged);
        assert!(!decoded(garbled.as_bytes(), false).repaired);
        assert!(!decoded(b"ascii", true).repaired);
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
