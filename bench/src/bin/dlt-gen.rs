// Synthetic DLT files, the same for the same size and kind.
// `android`: verbose string and integer arguments with Android-like text,
// levels V 20 / D 30 / I 35 / W 8 / E 5 / F 2 %, 3 ECUs x 40 APIDs x 200
// CTIDs, all optional header fields, 10 % big-endian payloads and
// `RarE_NEEDLE_<i>` every 1,024 messages.
// `ios`: iOS-like bracketed component text, booleans and doubles, about 30 %
// Korean and emoji, 5 % raw data of 16-64 bytes.
// `stress`: the Android mix plus 15 % non-verbose and 5 % control messages,
// 1 % 4 KiB raw data, 1-8 KiB of other bytes every ~10 MiB and a message cut
// short at the end.
// With `--twin <path>` it also writes the rendered rows (UTC) as a text log,
// for comparing the same rows read as text.
// Usage: dlt-gen <MiB> <output.dlt> [android|ios|stress] [--twin <path.log>]
use logstudio_core::dlt::{
    synth::{id, msin, verbose, with_id, Arg, Message},
    Clock, Decoder,
};
use std::io::Write;
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}
const WORDS: [&str; 12] = [
    "connection",
    "state",
    "changed",
    "request",
    "timeout",
    "buffer",
    "route",
    "volume",
    "session",
    "update",
    "value",
    "done",
];
const KOREAN: [&str; 4] = ["한글 텍스트", "경로 재탐색", "음량 변경 🙂", "연결 완료 🚗"];
fn level(r: &mut Rng) -> u8 {
    // 1 fatal ... 6 verbose.
    match r.next(100) {
        0..=1 => 1,
        2..=6 => 2,
        7..=14 => 3,
        15..=49 => 4,
        50..=79 => 5,
        _ => 6,
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mib: usize = a[1].parse().unwrap();
    let kind = a.get(3).map_or("android", |k| k.as_str());
    let twin = a
        .iter()
        .position(|x| x == "--twin")
        .map(|i| a[i + 1].clone());
    let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&a[2]).unwrap());
    let mut t =
        twin.map(|p| std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(p).unwrap()));
    let decoder = Decoder::new(Clock::Fixed(0));
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut n, mut i, mut garbage_at) = (0usize, 0u64, 10usize << 20);
    let (mut buf, mut rows) = (Vec::with_capacity(1 << 16), Vec::with_capacity(1 << 16));
    let raw: Vec<u8> = (0..4096).map(|k| (k * 7) as u8).collect();
    while n < mib << 20 {
        buf.clear();
        let apid = format!("A{:02}", r.next(40));
        let ctid = format!("C{:03}", r.next(200));
        let text = match kind {
            "ios" => {
                if r.next(10) < 3 {
                    format!(
                        "[SDK_UI] [GalleryManager] {} message {i}",
                        KOREAN[r.next(4) as usize]
                    )
                } else {
                    format!("[SDK_UI] [Comp{}] message {i} payload=abcdef{i}", r.next(7))
                }
            }
            _ => {
                let w = |r: &mut Rng| WORDS[r.next(WORDS.len() as u64) as usize];
                if i % 1024 == 7 {
                    format!("RarE_NEEDLE_{i} found")
                } else {
                    format!(
                        "{} {} {} number {i} with data",
                        w(&mut r),
                        w(&mut r),
                        w(&mut r)
                    )
                }
            }
        };
        let rawd = &raw[..16 + r.next(49) as usize];
        let mut m = match kind {
            "ios" if r.next(20) == 0 => Message::log(
                level(&mut r),
                &apid,
                &ctid,
                &[Arg::Utf8(&text), Arg::Raw(rawd)],
            ),
            "ios" => Message::log(
                level(&mut r),
                &apid,
                &ctid,
                &[
                    Arg::Utf8(&text),
                    Arg::Bool(i % 2 == 0),
                    Arg::F64(i as f64 / 7.0),
                ],
            ),
            _ => {
                let args = [
                    Arg::Ascii(text.as_bytes()),
                    Arg::Uint(i * 7, 4),
                    Arg::Sint(-(i as i64 % 1000), 2),
                ];
                let m = Message::log(level(&mut r), &apid, &ctid, &args);
                if r.next(10) == 0 {
                    m.big(&args)
                } else {
                    m
                }
            }
        };
        m.seconds = 1_773_480_413 + (i / 1000) as u32;
        m.micros = (i % 1000) as u32 * 1000 + r.next(1000) as u32;
        m.count = i as u8;
        m.ecu = Some(id(["ECU1", "HU", "TCU"][r.next(3) as usize]));
        m.session = Some(1000 + r.next(64) as u32);
        m.timestamp = Some((i * 37) as u32);
        if kind == "stress" {
            match r.next(100) {
                0..=14 => {
                    let (_, _, a, c) = m.ext.unwrap();
                    m.ext = Some((msin(false, 0, 4), 0, a, c));
                    m.payload = with_id(
                        m.big_endian,
                        r.next(5000) as u32,
                        &raw[..r.next(64) as usize],
                    );
                }
                15..=19 => {
                    m.ext = Some((msin(false, 3, 1 + r.next(2) as u8), 0, id("DA1"), id("DC1")));
                    m.payload = with_id(m.big_endian, 1 + r.next(20) as u32, &[0, 7, 1, 2]);
                }
                20 => {
                    m.payload = verbose(m.big_endian, &[Arg::Raw(&raw)]);
                    m.ext = m.ext.map(|(msin, _, a, c)| (msin, 1, a, c));
                }
                _ => {}
            }
        }
        m.write(&mut buf);
        if kind == "stress" && n >= garbage_at {
            buf.extend((0..1024 + r.next(7 * 1024)).map(|k| (k * 13 + 5) as u8));
            garbage_at += 10 << 20;
        }
        f.write_all(&buf).unwrap();
        if let Some(t) = t.as_mut() {
            rows.clear();
            decoder.render(&buf, n as u64, &mut rows);
            t.write_all(&rows).unwrap();
        }
        n += buf.len();
        i += 1;
    }
    if kind == "stress" {
        let cut = Message::log(4, "CUT", "END", &[Arg::Ascii(b"cut short")]).bytes();
        let cut = &cut[..cut.len() - 5];
        f.write_all(cut).unwrap();
        if let Some(t) = t.as_mut() {
            rows.clear();
            decoder.render(cut, n as u64, &mut rows);
            t.write_all(&rows).unwrap();
        }
    }
    f.flush().unwrap();
    if let Some(mut t) = t {
        t.flush().unwrap();
    }
    eprintln!("{i} messages");
}
