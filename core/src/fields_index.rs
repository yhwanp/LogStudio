//! Per-block PID, TID and tag metadata recorded by the analysis pass. A scan
//! skips blocks in which no row can pass the showing conditions, and counts a
//! block without reading it when levels and at most one of these fields
//! decide it.
//!
//! Each field keeps a dictionary of the log's distinct values (tags as the
//! tag sets of parsed rows) and, per block and line number variant, its rows
//! by (value, level slot). Rows without fields count with the entry they
//! continue, as in the level table: a block's leading rows with the last
//! entry of an earlier block of the same file, orphan rows nowhere. Each
//! condition is tested once per query against each dictionary, only through
//! the `Cond` predicates, so the decision stays that of `fields_match`.
use crate::{
    dataset::BlockRows,
    parser::Record,
    search::{self, Cond, Filter},
};
use std::{collections::HashMap, sync::Arc};

/// Field indexes, also of `Filter::tested_fields`.
pub const PID: usize = 0;
pub const TID: usize = 1;
pub const TAGS: usize = 2;
/// Distinct values or tag sets one field keeps; more drop its metadata.
pub const MAX_VALUES: usize = 65_536;
/// Approximate memory one field's metadata may use; more drop it. Scans that
/// test a dropped field read its blocks as without metadata.
pub const MAX_FIELD_BYTES: usize = 32 << 20;
const NONE: u32 = u32::MAX;
/// PID, TID and tag set ids of one parsed entry.
type Key = [u32; 3];

/// The multiply-rotate hash of rustc's `FxHasher` for short field values,
/// much cheaper than SipHash per row. Its maps hold one block's values or a
/// capped dictionary, which bounds the cost of colliding input.
#[derive(Default)]
struct Fx(u64);
impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u8(&mut self, n: u8) {
        self.write_u64(u64::from(n));
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0 ^ n).wrapping_mul(0xf135_7aea_2e62_a9c5);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }
}
type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;
/// One block's distinct values with block-local ids.
struct Values<'a> {
    /// IDs of up to 17 digits by value and length, so that a leading zero
    /// still makes another value; other values by text.
    digits: FxMap<u64, u32>,
    ids: FxMap<&'a str, u32>,
    list: Vec<&'a str>,
    /// The previous row's value; a thread often logs several rows in a row.
    last: Option<(&'a str, u32)>,
}
impl Default for Values<'_> {
    fn default() -> Self {
        // A block's rows usually carry tens to hundreds of distinct values.
        Self {
            digits: FxMap::with_capacity_and_hasher(128, Default::default()),
            ids: FxMap::with_capacity_and_hasher(128, Default::default()),
            list: Vec::with_capacity(128),
            last: None,
        }
    }
}
impl<'a> Values<'a> {
    fn id(&mut self, s: &'a str) -> u32 {
        if let Some((_, id)) = self.last.filter(|&(last, _)| last == s) {
            return id;
        }
        let next = self.list.len() as u32;
        let b = s.as_bytes();
        // Most rows repeat a value of the block; `entry` would reserve first.
        let id = if (1..=17).contains(&b.len()) && b.iter().all(u8::is_ascii_digit) {
            let value = b.iter().fold(0, |v, &d| v * 10 + u64::from(d - b'0'));
            let key = value << 5 | b.len() as u64;
            match self.digits.get(&key) {
                Some(&id) => id,
                None => *self.digits.entry(key).or_insert(next),
            }
        } else {
            match self.ids.get(s) {
                Some(&id) => id,
                None => *self.ids.entry(s).or_insert(next),
            }
        };
        if id == next {
            self.list.push(s);
        }
        self.last = Some((s, id));
        id
    }
}
/// Rows of one block for one line number variant, by local ids.
#[derive(Clone, Default)]
struct Hist {
    /// Rows per local id and level slot, for each field.
    rows: [Vec<[u32; 8]>; 3],
    /// Slot and key of the entry being counted; None for an orphan entry.
    open: Option<(u8, Key)>,
    run: u32,
    /// Whether an entry row was seen; rows before it lead the block.
    seen: bool,
    lead: u32,
}
impl Hist {
    fn add(&mut self, opens: Option<usize>, key: Option<Key>) {
        match opens {
            Some(slot) => {
                self.flush();
                self.open = key.map(|k| (slot as u8, k));
                self.run = 1;
                self.seen = true;
            }
            None if self.seen => self.run += 1,
            None => self.lead += 1,
        }
    }
    fn flush(&mut self) {
        if let Some((slot, key)) = self.open {
            for (rows, &id) in self.rows.iter_mut().zip(&key) {
                let id = id as usize;
                if rows.len() <= id {
                    rows.resize(id + 1, [0; 8]);
                }
                rows[id][slot as usize] += self.run;
            }
        }
        self.run = 0;
    }
    fn finish(mut self) -> Counted {
        self.flush();
        Counted {
            pairs: self.rows.map(|rows| {
                let mut pairs = vec![];
                for (id, slots) in rows.iter().enumerate() {
                    for (slot, &n) in slots.iter().enumerate() {
                        if n > 0 {
                            pairs.push((id as u32, slot as u8, n));
                        }
                    }
                }
                pairs
            }),
            lead: self.lead,
            tail: self.seen.then_some(self.open),
        }
    }
}
/// Collects one block's field values while the analysis parses its rows.
#[derive(Default)]
pub struct Collector<'a> {
    values: [Values<'a>; 3],
    /// Local tag set of the rows with one tag, by local tag id.
    single: Vec<u32>,
    sets: FxMap<Vec<u32>, u32>,
    set_tags: Vec<u32>,
    set_ends: Vec<u32>,
    scratch: Vec<u32>,
    hist: Hist,
    /// Rows with line number recognition, from the first row on which it
    /// differs; until then both variants are `hist`.
    numbered: Option<Hist>,
}
impl<'a> Collector<'a> {
    /// Adds the next row: its record without line number recognition and,
    /// when recognition parses it differently, with it, each with the level
    /// slot of the entry it opens (8 for an orphan entry, None for a row that
    /// continues the entry before it).
    pub fn add(
        &mut self,
        plain: &Record<'a>,
        opens: Option<usize>,
        numbered: Option<&Record<'a>>,
        opens_numbered: Option<usize>,
    ) {
        let key = self.key(plain, opens);
        let other = match numbered {
            Some(r) => self.key(r, opens_numbered),
            None => key,
        };
        if self.numbered.is_none() && (numbered.is_some() || opens != opens_numbered) {
            self.numbered = Some(self.hist.clone());
        }
        self.hist.add(opens, key);
        if let Some(h) = &mut self.numbered {
            h.add(opens_numbered, other);
        }
    }
    fn key(&mut self, r: &Record<'a>, opens: Option<usize>) -> Option<Key> {
        let p = r.parsed.as_ref().filter(|_| opens.is_some_and(|s| s < 8))?;
        let set = match search::row_tags(p.tag_list()) {
            [tag] => {
                let t = self.values[TAGS].id(tag) as usize;
                if self.single.len() <= t {
                    self.single.resize(t + 1, NONE);
                }
                if self.single[t] == NONE {
                    self.single[t] = self.new_set(&[t as u32]);
                }
                self.single[t]
            }
            tags => {
                let mut scratch = std::mem::take(&mut self.scratch);
                scratch.clear();
                scratch.extend(tags.iter().map(|t| self.values[TAGS].id(t)));
                let id = match self.sets.get(&scratch) {
                    Some(&id) => id,
                    None => {
                        let id = self.new_set(&scratch);
                        self.sets.insert(scratch.clone(), id);
                        id
                    }
                };
                self.scratch = scratch;
                id
            }
        };
        Some([self.values[PID].id(p.pid), self.values[TID].id(p.tid), set])
    }
    fn new_set(&mut self, tags: &[u32]) -> u32 {
        self.set_tags.extend_from_slice(tags);
        self.set_ends.push(self.set_tags.len() as u32);
        self.set_ends.len() as u32 - 1
    }
    pub fn finish(self) -> Block {
        Block {
            values: self.values.map(|v| {
                let mut s = Strings::default();
                for value in v.list {
                    s.text.push_str(value);
                    s.ends.push(s.text.len() as u32);
                }
                s
            }),
            set_tags: self.set_tags,
            set_ends: self.set_ends,
            plain: self.hist.finish(),
            numbered: self.numbered.map(Hist::finish),
        }
    }
}
#[derive(Default)]
struct Strings {
    text: String,
    ends: Vec<u32>,
}
impl Strings {
    fn get(&self, i: usize) -> &str {
        let start = i.checked_sub(1).map_or(0, |p| self.ends[p] as usize);
        &self.text[start..self.ends[i] as usize]
    }
}
/// One variant's rows of a block by (local id, slot), for each field.
struct Counted {
    pairs: [Vec<(u32, u8, u32)>; 3],
    /// Rows before the block's first entry row.
    lead: u32,
    /// Slot and key of the block's last entry (None for an orphan), if any.
    tail: Option<Option<(u8, Key)>>,
}
/// One block's values with block-local ids, from a worker to the builder.
pub struct Block {
    values: [Strings; 3],
    set_tags: Vec<u32>,
    set_ends: Vec<u32>,
    plain: Counted,
    /// None when line number recognition changes no row of the block.
    numbered: Option<Counted>,
}
impl Block {
    fn set(&self, i: usize) -> &[u32] {
        let start = i.checked_sub(1).map_or(0, |p| self.set_ends[p] as usize);
        &self.set_tags[start..self.set_ends[i] as usize]
    }
}

#[derive(Clone)]
struct FieldBlock {
    /// Id of the block's last entry when it is parsed, per variant.
    tail: [u32; 2],
    /// Rows by (id, level slot) without line number recognition, and with it
    /// when they differ.
    plain: Box<[u8]>,
    numbered: Option<Box<[u8]>>,
}
#[derive(Clone, Default)]
struct Field {
    /// PIDs, TIDs or tags by id.
    values: Vec<Box<str>>,
    /// Tag sets by id, as sorted tag ids; tags only.
    sets: Vec<Box<[u32]>>,
    blocks: Vec<FieldBlock>,
    /// Approximate memory use.
    bytes: usize,
}
impl Field {
    /// Whether each value (each tag set) passes the condition's test of this
    /// field.
    fn verdicts(&self, f: usize, c: &Cond) -> Vec<bool> {
        match f {
            PID => self.values.iter().map(|v| c.accepts_pid(v)).collect(),
            TID => self.values.iter().map(|v| c.accepts_tid(v)).collect(),
            _ => {
                let tags: Vec<bool> = self.values.iter().map(|t| c.accepts_tag(t)).collect();
                self.sets
                    .iter()
                    .map(|s| s.iter().any(|&t| tags[t as usize]))
                    .collect()
            }
        }
    }
}
/// A field's dictionary while the analysis runs.
#[derive(Default)]
struct Build {
    field: Field,
    ids: FxMap<Box<str>, u32>,
    sets: FxMap<Box<[u32]>, u32>,
}
impl Build {
    fn value(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.field.values.len() as u32;
        let s: Box<str> = s.into();
        // The string twice, two boxes and a map slot.
        self.field.bytes += 2 * s.len() + 48;
        self.ids.insert(s.clone(), id);
        self.field.values.push(s);
        id
    }
    fn set(&mut self, tags: &[u32]) -> u32 {
        if let Some(&id) = self.sets.get(tags) {
            return id;
        }
        let id = self.field.sets.len() as u32;
        let s: Box<[u32]> = tags.into();
        self.field.bytes += 8 * s.len() + 48;
        self.sets.insert(s.clone(), id);
        self.field.sets.push(s);
        id
    }
    fn full(&self) -> bool {
        self.field.values.len() > MAX_VALUES
            || self.field.sets.len() > MAX_VALUES
            || self.field.bytes > MAX_FIELD_BYTES
    }
}
/// Joins the blocks' values into the dictionaries in block order.
pub struct Builder {
    fields: [Option<Build>; 3],
    /// Per variant, the parsed entry that leading rows of the next block of
    /// the same file continue; None at a file's start or after an orphan.
    carry: [Option<(u8, Key)>; 2],
    file: Option<usize>,
}
impl Default for Builder {
    fn default() -> Self {
        Self {
            fields: std::array::from_fn(|_| Some(Build::default())),
            carry: [None; 2],
            file: None,
        }
    }
}
impl Builder {
    pub fn push(&mut self, file: usize, block: Block) {
        if self.file != Some(file) {
            (self.file, self.carry) = (Some(file), [None; 2]);
        }
        // Global ids of the block's local ids; empty for a dropped field.
        let mut ids: [Vec<u32>; 3] = Default::default();
        for (f, field) in self.fields.iter_mut().enumerate() {
            let Some(b) = field else { continue };
            let values = &block.values[f];
            let local: Vec<u32> = (0..values.ends.len())
                .map(|i| b.value(values.get(i)))
                .collect();
            ids[f] = if f == TAGS {
                let mut tags = vec![];
                (0..block.set_ends.len())
                    .map(|s| {
                        tags.clear();
                        tags.extend(block.set(s).iter().map(|&t| local[t as usize]));
                        tags.sort_unstable();
                        b.set(&tags)
                    })
                    .collect()
            } else {
                local
            };
        }
        let global = |key: Key| -> Key {
            std::array::from_fn(|f| ids[f].get(key[f] as usize).copied().unwrap_or(NONE))
        };
        let counted = [
            &block.plain,
            block.numbered.as_ref().unwrap_or(&block.plain),
        ];
        let tails = counted.map(|c| c.tail.map(|t| t.map(|(slot, key)| (slot, global(key)))));
        // Leading rows count with the entry they continue, which may differ
        // between the variants even when the block's rows do not.
        let same =
            block.numbered.is_none() && (block.plain.lead == 0 || self.carry[0] == self.carry[1]);
        let carry = self.carry;
        for (f, field) in self.fields.iter_mut().enumerate() {
            let Some(b) = field else { continue };
            let rows = |v: usize| {
                let c = counted[v];
                let mut pairs: Vec<(u32, u8, u32)> = c.pairs[f]
                    .iter()
                    .map(|&(id, slot, n)| (ids[f][id as usize], slot, n))
                    .collect();
                if let (true, Some((slot, key))) = (c.lead > 0, carry[v]) {
                    pairs.push((key[f], slot, c.lead));
                }
                encode(&mut pairs)
            };
            let plain = rows(0);
            let numbered = (!same).then(|| rows(1));
            let tail = tails.map(|t| match t {
                Some(Some((_, key))) => key[f],
                _ => NONE,
            });
            b.field.bytes += plain.len()
                + numbered.as_ref().map_or(0, |n| n.len())
                + std::mem::size_of::<FieldBlock>();
            b.field.blocks.push(FieldBlock {
                tail,
                plain,
                numbered,
            });
            if b.full() {
                *field = None;
            }
        }
        for (carry, tail) in self.carry.iter_mut().zip(tails) {
            if let Some(tail) = tail {
                *carry = tail;
            }
        }
    }
    pub fn finish(self) -> FieldIndex {
        FieldIndex {
            fields: self.fields.map(|b| {
                b.map(|b| {
                    let mut field = b.field;
                    field.blocks.shrink_to_fit();
                    field
                })
            }),
        }
    }
}
/// Sorted (id, slot, rows) as LEB128 `(id delta << 4 | slot), rows`.
fn encode(pairs: &mut [(u32, u8, u32)]) -> Box<[u8]> {
    pairs.sort_unstable_by_key(|&(id, slot, _)| (id, slot));
    let mut out = Vec::with_capacity(pairs.len() * 3);
    let mut put = |mut v: u64| loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    };
    let (mut last, mut i) = (0, 0);
    while i < pairs.len() {
        let (id, slot, mut n) = pairs[i];
        i += 1;
        // A tag set seen in two orders gets one id; merge its rows.
        while pairs.get(i).is_some_and(|p| (p.0, p.1) == (id, slot)) {
            n += pairs[i].2;
            i += 1;
        }
        put((u64::from(id - last) << 4) | u64::from(slot));
        put(u64::from(n));
        last = id;
    }
    // An exact allocation: shrinking `out` in place may keep its capacity.
    out.as_slice().into()
}
/// Decodes `encode`: (id, slot, rows).
struct Pairs<'a> {
    data: &'a [u8],
    at: usize,
    id: u32,
}
impl Pairs<'_> {
    fn varint(&mut self) -> u64 {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let byte = self.data[self.at];
            self.at += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return v;
            }
            shift += 7;
        }
    }
}
impl Iterator for Pairs<'_> {
    type Item = (u32, usize, u32);
    fn next(&mut self) -> Option<Self::Item> {
        if self.at == self.data.len() {
            return None;
        }
        let a = self.varint();
        let n = self.varint() as u32;
        self.id += (a >> 4) as u32;
        Some((self.id, (a & 15) as usize, n))
    }
}
fn pairs(data: &[u8]) -> Pairs<'_> {
    Pairs { data, at: 0, id: 0 }
}

/// Field metadata of every analyzed block, published with the level table.
#[derive(Clone)]
pub struct FieldIndex {
    /// PID, TID and tag metadata; None once dropped.
    fields: [Option<Field>; 3],
}
impl FieldIndex {
    /// Blocks described by the metadata still kept.
    pub fn blocks(&self) -> usize {
        self.fields
            .iter()
            .flatten()
            .map(|f| f.blocks.len())
            .max()
            .unwrap_or(0)
    }
    /// Forgets the blocks from `k` on, as a followed file's replaced last
    /// block is; scans read blocks the metadata no longer describes.
    pub fn truncate(&mut self, k: usize) {
        for field in self.fields.iter_mut().flatten() {
            field.blocks.truncate(k);
        }
    }
    /// Whether the PID, TID and tag metadata were kept.
    pub fn kept(&self) -> [bool; 3] {
        std::array::from_fn(|f| self.fields[f].is_some())
    }
    /// Approximate memory of the PID, TID and tag metadata, if kept.
    pub fn bytes(&self) -> [usize; 3] {
        std::array::from_fn(|f| self.fields[f].as_ref().map_or(0, |x| x.bytes))
    }
    /// Distinct values of each kept field (tag sets for tags).
    pub fn values(&self) -> [Option<usize>; 3] {
        std::array::from_fn(|f| {
            self.fields[f].as_ref().map(|x| {
                if f == TAGS {
                    x.sets.len()
                } else {
                    x.values.len()
                }
            })
        })
    }
    /// Rows a kept field's metadata counts in one block and variant: every
    /// row except orphan rows.
    pub fn counted_rows(&self, field: usize, block: usize, variant: usize) -> Option<u64> {
        let b = self.fields.get(field)?.as_ref()?.blocks.get(block)?;
        Some(pairs(b.rows(variant)).map(|(_, _, n)| u64::from(n)).sum())
    }
    /// A condition's tested fields evaluated over their dictionaries. A
    /// field that accepts every value of the log decides nothing and is
    /// left out; a dropped one has no verdicts.
    fn known(&self, c: &Cond) -> Known {
        let mut fields = vec![];
        for (f, tested) in c.tests().into_iter().enumerate() {
            if !tested {
                continue;
            }
            match &self.fields[f] {
                Some(field) => {
                    let verdicts = field.verdicts(f, c);
                    if verdicts.contains(&false) {
                        fields.push((f, Some(verdicts)));
                    }
                }
                None => fields.push((f, None)),
            }
        }
        Known {
            levels: c.level_bits(),
            fields,
            tested: c.tests(),
        }
    }
    /// Whether the kept metadata of each `tested` field describes `block`.
    /// A followed file's later blocks hold values its dictionary may lack,
    /// so there a test that every known value passes, or an empty
    /// dictionary's, says nothing. A dropped field reads its blocks anyway.
    fn covers(&self, tested: [bool; 3], block: usize) -> bool {
        self.fields
            .iter()
            .zip(tested)
            .all(|(field, t)| !t || field.as_ref().is_none_or(|f| block < f.blocks.len()))
    }
    /// Each condition's tested fields evaluated over their dictionaries;
    /// None when the metadata cannot decide any block.
    pub fn plan(self: &Arc<Self>, filter: &Filter) -> Option<Plan> {
        let conds: Vec<Known> = filter.conditions().map(|c| self.known(c)).collect();
        let fields = || conds.iter().flat_map(|k| &k.fields);
        let exact = filter.only_fields() && fields().all(|(_, v)| v.is_some());
        let mut useful = fields().any(|(_, v)| v.is_some());
        // Levels alone can show that no row passes a showing condition.
        useful |= !filter.shows().is_empty();
        (useful || exact).then(|| Plan {
            table: self.clone(),
            conds,
            shows: filter.shows().len(),
            exact,
            orphan: filter.admits_unparsed(),
        })
    }
}
/// One condition's use of the field metadata, to learn whether rows of a
/// block match it without reading them.
pub struct Probe {
    table: Arc<FieldIndex>,
    known: Known,
    /// Whether levels and fields decide the condition: no time range and no
    /// tested field without metadata.
    exact: bool,
}
impl FieldIndex {
    pub fn probe(self: &Arc<Self>, c: &Cond) -> Probe {
        let known = self.known(c);
        let exact = !c.has_time() && known.fields.iter().all(|(_, v)| v.is_some());
        Probe {
            table: self.clone(),
            known,
            exact,
        }
    }
}
impl Probe {
    /// Some(true) when a row of `block` matches the condition, Some(false)
    /// when none does, None when only its rows can tell: a time range, two
    /// fields that each pass some rows, or no metadata for the block. `rows`
    /// is the block's level table entry. Rows count with their entry, so a
    /// matching row means a matching entry.
    pub fn block(&self, block: usize, variant: usize, rows: &BlockRows) -> Option<bool> {
        let in_levels = |slot: usize| self.known.levels & (1 << slot) != 0;
        if (0..8).all(|slot| !in_levels(slot) || rows.slots[slot] == 0) {
            return Some(false);
        }
        if !self.table.covers(self.known.tested, block) {
            return None;
        }
        let mut deciding = 0;
        for (f, verdicts) in &self.known.fields {
            let Some(verdicts) = verdicts else {
                continue;
            };
            let b = self.table.fields[*f].as_ref()?.blocks.get(block)?;
            let (mut any, mut all) = (false, true);
            for (id, slot, _) in pairs(b.rows(variant)) {
                if !in_levels(slot) {
                    continue;
                }
                if verdicts[id as usize] {
                    any = true;
                } else {
                    all = false;
                }
                if any && !all {
                    break;
                }
            }
            if !any {
                return Some(false);
            }
            deciding += usize::from(!all);
        }
        // Rows of the levels pass every other field, so one passing a
        // single deciding field matches.
        (self.exact && deciding <= 1).then_some(true)
    }
}
impl FieldBlock {
    fn rows(&self, variant: usize) -> &[u8] {
        match (variant, &self.numbered) {
            (1, Some(n)) => n,
            _ => &self.plain,
        }
    }
}
/// One query's use of the field metadata.
pub struct Plan {
    table: Arc<FieldIndex>,
    /// Showing conditions, then hiding ones.
    conds: Vec<Known>,
    shows: usize,
    /// Whether metadata decides every row: no time range, search query or
    /// tested field without metadata.
    exact: bool,
    /// Whether orphan rows pass.
    orphan: bool,
}
/// Fields that still decide a condition in a block, with their verdicts.
type Deciding<'a> = Vec<(usize, &'a [bool])>;
/// One condition as the metadata sees it.
struct Known {
    /// Bits over the level slots it matches.
    levels: u16,
    /// Each field it tests that rejects some value, with a verdict per id;
    /// None for a field without metadata.
    fields: Vec<(usize, Option<Vec<bool>>)>,
    /// The fields it tests, PID, TID and tags (`Cond::tests`).
    tested: [bool; 3],
}
impl Plan {
    /// The block's matches and its last entry's decision, when field
    /// metadata decides them without reading: no row can pass a showing
    /// condition, or levels and at most one field decide every row. `rows` is
    /// the block's level table entry.
    pub fn decide(
        &self,
        block: usize,
        variant: usize,
        rows: &BlockRows,
    ) -> Option<(u64, Option<bool>)> {
        if self
            .conds
            .iter()
            .any(|c| !self.table.covers(c.tested, block))
        {
            return None;
        }
        let in_levels = |levels: u16, slot: usize| levels & (1 << slot) != 0;
        // Per condition: None when no row of the block matches it, else the
        // fields that still decide it here. A field whose every value in the
        // block passes decides nothing in it.
        let mut states: Vec<Option<Deciding>> = Vec::with_capacity(self.conds.len());
        for c in &self.conds {
            if (0..8).all(|slot| !in_levels(c.levels, slot) || rows.slots[slot] == 0) {
                states.push(None);
                continue;
            }
            let (mut deciding, mut dead) = (vec![], false);
            for (f, verdicts) in &c.fields {
                let Some(verdicts) = verdicts else {
                    continue;
                };
                let b = self.table.fields[*f].as_ref()?.blocks.get(block)?;
                let (mut any, mut all) = (false, true);
                for (id, slot, _) in pairs(b.rows(variant)) {
                    if !in_levels(c.levels, slot) {
                        continue;
                    }
                    if verdicts[id as usize] {
                        any = true;
                    } else {
                        all = false;
                    }
                }
                if !any {
                    dead = true;
                    break;
                }
                if !all {
                    deciding.push((*f, verdicts.as_slice()));
                }
            }
            states.push((!dead).then_some(deciding));
        }
        if self.shows > 0 && states[..self.shows].iter().all(Option::is_none) {
            // No entry of the block shows, and with a showing condition no
            // orphan does either; also the last entry fails.
            return Some((0, rows.tail.map(|_| false)));
        }
        if !self.exact {
            return None;
        }
        // At most one field may still decide; its rows by (id, slot) count.
        let mut field = None;
        for (f, _) in states.iter().flatten().flatten() {
            if field.is_some_and(|g| g != *f) {
                return None;
            }
            field = Some(*f);
        }
        let matches = |i: usize, slot: usize, id: Option<u32>| {
            states[i].as_ref().is_some_and(|deciding| {
                in_levels(self.conds[i].levels, slot)
                    && deciding
                        .iter()
                        .all(|(_, v)| id.is_some_and(|id| v[id as usize]))
            })
        };
        let pass = |slot: usize, id: Option<u32>| {
            (self.shows == 0 || (0..self.shows).any(|i| matches(i, slot, id)))
                && !(self.shows..self.conds.len()).any(|i| matches(i, slot, id))
        };
        let orphans = if self.orphan {
            u64::from(rows.slots[8])
        } else {
            0
        };
        let (parsed, tail_id): (u64, _) = match field {
            None => (
                (0..8)
                    .filter(|&slot| pass(slot, None))
                    .map(|slot| u64::from(rows.slots[slot]))
                    .sum(),
                None,
            ),
            Some(f) => {
                let b = self.table.fields[f].as_ref()?.blocks.get(block)?;
                (
                    pairs(b.rows(variant))
                        .filter(|&(id, slot, _)| pass(slot, Some(id)))
                        .map(|(_, _, n)| u64::from(n))
                        .sum(),
                    Some(b.tail[variant]),
                )
            }
        };
        let tail = rows.tail.map(|slot| {
            if slot == 8 {
                self.orphan
            } else {
                pass(slot as usize, tail_id)
            }
        });
        Some((orphans + parsed, tail))
    }
}
