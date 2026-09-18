use pixelgen_core::encode::{webm, Chunk};

/// One parsed element: its ID with the length marker kept, and its body.
struct El<'a> {
    id: u32,
    body: &'a [u8],
}

fn vint(b: &[u8], keep_marker: bool) -> (u64, usize) {
    let n = b[0].leading_zeros() as usize + 1;
    let mut v = if keep_marker { b[0] as u64 } else { (b[0] as u64) & ((1 << (8 - n)) - 1) };
    for &x in &b[1..n] {
        v = v << 8 | x as u64;
    }
    (v, n)
}

/// The elements laid end to end in `b`, which must be exactly covered.
fn parse(mut b: &[u8]) -> Vec<El<'_>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let (id, a) = vint(b, true);
        let (len, c) = vint(&b[a..], false);
        let end = a + c + len as usize;
        out.push(El { id: id as u32, body: &b[a + c..end] });
        b = &b[end..];
    }
    out
}

fn child<'a>(els: &[El<'a>], id: u32) -> &'a [u8] {
    els.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no element {id:X}")).body
}

fn uint(b: &[u8]) -> u64 {
    b.iter().fold(0, |v, &x| v << 8 | x as u64)
}

/// Every block as (absolute ms, key, payload), and the cluster count.
fn blocks(file: &[u8]) -> (Vec<(u64, bool, Vec<u8>)>, usize) {
    let top = parse(file);
    let segment = parse(child(&top, 0x1853_8067));
    let mut out = Vec::new();
    let mut clusters = 0;
    for c in segment.iter().filter(|e| e.id == 0x1F43_B675) {
        clusters += 1;
        let els = parse(c.body);
        let base = uint(child(&els, 0xE7));
        for b in els.iter().filter(|e| e.id == 0xA3) {
            assert_eq!(b.body[0], 0x81, "track number");
            let rel = i16::from_be_bytes([b.body[1], b.body[2]]);
            assert!(rel >= 0);
            out.push((base + rel as u64, b.body[3] & 0x80 != 0, b.body[4..].to_vec()));
        }
    }
    (out, clusters)
}

fn frames(n: usize, key_every: usize) -> Vec<(Vec<u8>, bool)> {
    (0..n).map(|i| (vec![i as u8; 3 + i % 5], i % key_every == 0)).collect()
}

fn chunks(f: &[(Vec<u8>, bool)]) -> Vec<Chunk<'_>> {
    f.iter().map(|(d, k)| Chunk { data: d, key: *k }).collect()
}

#[test]
fn webm_is_an_ebml_webm_document() {
    let f = frames(4, 2);
    let out = webm(1200, 800, 12, &chunks(&f)).unwrap();
    assert_eq!(&out[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    let top = parse(&out);
    assert_eq!(top.len(), 2, "header and segment, nothing trailing");
    assert_eq!(child(&parse(child(&top, 0x1A45_DFA3)), 0x4282), b"webm");

    let segment = parse(child(&top, 0x1853_8067));
    let track = parse(child(&parse(child(&segment, 0x1654_AE6B)), 0xAE));
    assert_eq!(child(&track, 0x86), b"V_VP9");
    let video = parse(child(&track, 0xE0));
    assert_eq!(uint(child(&video, 0xB0)), 1200);
    assert_eq!(uint(child(&video, 0xBA)), 800);
}

/// The whole point: the file is exactly frames / fps long, whatever the
/// encoder's pace was.
#[test]
fn webm_duration_and_timestamps_follow_the_frame_rate() {
    let f = frames(36, 12);
    let out = webm(64, 64, 12, &chunks(&f)).unwrap();
    let segment = parse(child(&parse(&out), 0x1853_8067));
    let info = parse(child(&segment, 0x1549_A966));
    assert_eq!(uint(child(&info, 0x2AD7B1)), 1_000_000);
    let d = f64::from_be_bytes(child(&info, 0x4489).try_into().unwrap());
    assert_eq!(d, 3000.0);

    let (b, clusters) = blocks(&out);
    assert_eq!(b.len(), 36);
    assert_eq!(clusters, 3, "one per key frame");
    for (i, (t, key, data)) in b.iter().enumerate() {
        assert_eq!(*t, (i as f64 * 1000.0 / 12.0).round() as u64);
        assert_eq!(*key, i % 12 == 0);
        assert_eq!(data, &f[i].0);
    }
}

#[test]
fn webm_splits_clusters_the_block_offset_cannot_reach() {
    // One key frame and 40 seconds of deltas: past what an i16 of
    // milliseconds can address from a single cluster start.
    let f = frames(400, usize::MAX);
    let out = webm(8, 8, 10, &chunks(&f)).unwrap();
    let (b, clusters) = blocks(&out);
    assert_eq!(b.len(), 400);
    assert!(clusters >= 2);
    assert!(b.iter().enumerate().all(|(i, (t, _, _))| *t == i as u64 * 100));
}

#[test]
fn webm_carries_frames_too_big_for_short_sizes() {
    let f = vec![(vec![7u8; 70_000], true), (vec![9u8; 200], false)];
    let (b, _) = blocks(&webm(8, 8, 24, &chunks(&f)).unwrap());
    assert_eq!(b[0].2.len(), 70_000);
    assert_eq!(b[1].2, vec![9u8; 200]);
}

#[test]
fn webm_refuses_what_it_cannot_play() {
    assert!(webm(8, 8, 12, &[]).is_err());
    assert!(webm(8, 8, 12, &[Chunk { data: &[1], key: false }]).is_err());
    assert!(webm(0, 8, 12, &[Chunk { data: &[1], key: true }]).is_err());
}
