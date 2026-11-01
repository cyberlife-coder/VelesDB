//! Unit tests for `DeltaBuffer`.

use super::DeltaBuffer;

#[test]
fn a_new_buffer_is_empty() {
    let buf = DeltaBuffer::new();
    assert!(buf.is_empty());
    assert!(buf.ids().is_empty());
    assert!(DeltaBuffer::default().is_empty());
}

#[test]
fn push_and_extend_buffer_entries() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0]);
    buf.extend([(2, vec![2.0]), (3, vec![3.0])]);
    assert_eq!(buf.len(), 3);
    assert_eq!(buf.ids(), vec![1, 2, 3]);
}

#[test]
fn a_second_write_of_an_id_replaces_the_first() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0]);
    buf.push(1, vec![2.0]);
    buf.extend([(1, vec![3.0]), (2, vec![4.0])]);
    assert_eq!(buf.len(), 2);
    assert_eq!(buf.entries(), vec![(1, vec![3.0]), (2, vec![4.0])]);
}

#[test]
fn remove_drops_an_id_and_ignores_an_absent_one() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    buf.remove(1);
    buf.remove(99);
    assert_eq!(buf.ids(), vec![2]);
}

/// A merge retires the entries it indexed; an entry written again since
/// keeps its newer vector and stays.
#[test]
fn retire_keeps_an_entry_written_again() {
    let buf = DeltaBuffer::new();
    buf.push(1, vec![1.0]);
    buf.push(2, vec![2.0]);
    let copied = buf.entries();
    buf.push(2, vec![9.0]);
    let indexed: Vec<(u64, &[f32])> = copied.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    buf.retire(&indexed);
    assert_eq!(buf.entries(), vec![(2, vec![9.0])]);
}

#[test]
fn concurrent_pushes_and_reads_see_every_entry() {
    let buf = std::sync::Arc::new(DeltaBuffer::new());
    let writers: Vec<_> = (0..4_u64)
        .map(|t| {
            let buf = std::sync::Arc::clone(&buf);
            std::thread::spawn(move || {
                for i in 0..50 {
                    buf.push(t * 1_000 + i, vec![0.0]);
                    let _ = buf.ids();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("test: writer");
    }
    assert_eq!(buf.len(), 200);
}
