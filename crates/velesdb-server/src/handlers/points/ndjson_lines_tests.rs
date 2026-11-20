use super::*;

/// Feeds `chunks` and returns the completed lines, the dropped count and the
/// unterminated tail.
fn split(max_line_bytes: usize, chunks: &[&[u8]]) -> (Vec<String>, usize, String) {
    let mut splitter = LineSplitter::new(max_line_bytes);
    let mut lines = Vec::new();
    let mut dropped = 0;
    for chunk in chunks {
        dropped += splitter.push(chunk, |line| {
            lines.push(String::from_utf8_lossy(line).into_owned());
        });
    }
    let tail = String::from_utf8_lossy(&splitter.finish()).into_owned();
    (lines, dropped, tail)
}

#[test]
fn lines_are_reassembled_across_chunk_boundaries() {
    let (lines, dropped, tail) = split(64, &[b"ab", b"c\nde", b"f\n\ngh"]);
    assert_eq!(lines, ["abc", "def", ""]);
    assert_eq!(dropped, 0);
    assert_eq!(tail, "gh");
}

#[test]
fn a_line_exactly_at_the_ceiling_is_kept() {
    let (lines, dropped, _) = split(4, &[b"abcd\n"]);
    assert_eq!(lines, ["abcd"]);
    assert_eq!(dropped, 0);
}

#[test]
fn an_oversized_line_is_dropped_once_and_the_next_line_survives() {
    // The long line crosses the ceiling in the second of four chunks.
    let (lines, dropped, tail) = split(4, &[b"ok\nxxx", b"xxx", b"xxx", b"x\nyes\n"]);
    assert_eq!(lines, ["ok", "yes"]);
    assert_eq!(dropped, 1);
    assert_eq!(tail, "");
}

#[test]
fn an_oversized_line_inside_one_chunk_is_dropped() {
    let (lines, dropped, _) = split(4, &[b"a\ntoo long\nb\n"]);
    assert_eq!(lines, ["a", "b"]);
    assert_eq!(dropped, 1);
}

/// The defect this type exists for: a body with no newline at all. Whatever
/// its length, the splitter holds none of it once the ceiling is crossed.
#[test]
fn a_body_without_a_newline_is_not_accumulated() {
    let mut splitter = LineSplitter::new(1024);
    let chunk = vec![b'a'; 4096];
    let mut dropped = 0;
    for _ in 0..256 {
        dropped += splitter.push(&chunk, |_| panic!("no line is ever completed"));
        assert!(splitter.pending.capacity() <= 1024);
    }
    assert_eq!(dropped, 1);
    assert!(splitter.finish().is_empty());
}
