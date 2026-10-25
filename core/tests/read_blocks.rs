//! `Dataset::read_blocks`, which app extensions read rows with: every block
//! once and in order, with the physical row numbers the page shows, over
//! LF, CRLF and CR rows alike; and stopping early.
use logstudio_core::{dataset::Dataset, source::ByteLines, source::Source};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn open(text: &[u8]) -> (tempfile::TempDir, Arc<Dataset>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.log");
    std::fs::write(&path, text).unwrap();
    let d = Dataset::new(1, Source::granted(&path).unwrap());
    d.start(|_| {});
    let deadline = Instant::now() + Duration::from_secs(20);
    while !d.index.lock().unwrap().done {
        assert!(Instant::now() < deadline, "index");
        thread::sleep(Duration::from_millis(1));
    }
    (dir, d)
}

#[test]
fn blocks_arrive_in_order_with_their_physical_rows() {
    // Enough rows for several blocks, ending rows three ways.
    let mut text = Vec::new();
    for i in 1..=12_000u32 {
        let ending: &[u8] = match i % 3 {
            0 => b"\n",
            1 => b"\r\n",
            _ => b"\r",
        };
        text.extend_from_slice(
            format!(
                "10-25 09:00:00.000  100  101 I Rows: row {i:05} {}",
                "x".repeat(20)
            )
            .as_bytes(),
        );
        text.extend_from_slice(ending);
    }
    let (_dir, d) = open(&text);
    let blocks = d.index.lock().unwrap().blocks.len();
    assert!(blocks > 2, "{blocks} blocks");
    let mut seen = vec![];
    let stop = AtomicBool::new(false);
    let next = d
        .read_blocks(
            0,
            &stop,
            |b, rows| {
                // Each row says its own number.
                ByteLines::new(&rows.text)
                    .enumerate()
                    .map(|(j, (_, row))| {
                        let at = row.windows(4).position(|w| w == b"row ").unwrap() + 4;
                        let n: u64 = std::str::from_utf8(&row[at..at + 5])
                            .unwrap()
                            .parse()
                            .unwrap();
                        (b.first + j as u64, n)
                    })
                    .collect::<Vec<_>>()
            },
            |i, rows| seen.push((i, rows)),
        )
        .unwrap();
    assert_eq!(next, blocks);
    assert_eq!(
        seen.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        (0..blocks).collect::<Vec<_>>()
    );
    let rows: Vec<(u64, u64)> = seen.into_iter().flat_map(|(_, rows)| rows).collect();
    assert_eq!(rows.len(), 12_000);
    assert!(
        rows.iter().all(|(line, n)| line == n),
        "row numbers match their text"
    );
    // From a later block, only the blocks after it.
    let mut later = vec![];
    assert_eq!(
        d.read_blocks(blocks - 1, &stop, |b, _| b.first, |i, _| later.push(i))
            .unwrap(),
        blocks
    );
    assert_eq!(later, [blocks - 1]);
}

#[test]
fn a_stop_ends_reading_early() {
    let text = "10-25 09:00:00.000  100  101 I Rows: a row of text\n".repeat(20_000);
    let (_dir, d) = open(text.as_bytes());
    let stop = AtomicBool::new(false);
    let mut delivered = 0;
    let next = d
        .read_blocks(
            0,
            &stop,
            |_, _| (),
            |_, ()| {
                delivered += 1;
                stop.store(true, Ordering::Release);
            },
        )
        .unwrap();
    assert!(delivered >= 1 && next == delivered, "{delivered} {next}");
    assert!(next < d.index.lock().unwrap().blocks.len());
}
