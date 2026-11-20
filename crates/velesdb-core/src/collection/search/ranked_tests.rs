//! Tests for `ranked`: the candidates an index scored, best first.

use std::cell::Cell;

use super::{comparisons, Ranked};

fn admitted(ranked: Ranked, n: usize, admit: impl Fn(u64) -> bool) -> Vec<(u64, f32)> {
    ranked.filter(|&(id, _)| admit(id)).take(n).collect()
}

#[test]
fn the_best_admitted_candidates_come_first() {
    let ranked = Ranked::new(vec![(1, 0.5), (2, 2.0), (3, 1.0), (4, 3.0)]);
    assert_eq!(admitted(ranked, 2, |id| id != 4), [(2, 2.0), (3, 1.0)]);
}

#[test]
fn the_walk_continues_where_it_stopped() {
    let mut ranked = Ranked::new(vec![(1, 0.5), (2, 2.0), (3, 1.0), (4, 3.0)]);
    assert_eq!(ranked.next(), Some((4, 3.0)));
    assert_eq!(
        ranked.by_ref().take(10).collect::<Vec<_>>(),
        [(2, 2.0), (3, 1.0), (1, 0.5)]
    );
    assert_eq!(ranked.next(), None);
}

/// A tie ranks by ascending id, whatever order the index scored the
/// candidates in; and admission is asked only until it has taken enough.
#[test]
fn ties_rank_by_ascending_id_and_admission_stops_at_n() {
    let ranked = Ranked::new(vec![(9, 1.0), (7, 1.0), (8, 1.0)]);
    let asked = std::cell::RefCell::new(Vec::new());
    let best = admitted(ranked, 2, |id| {
        asked.borrow_mut().push(id);
        true
    });
    assert_eq!(best, [(7, 1.0), (8, 1.0)]);
    assert_eq!(asked.into_inner(), [7, 8]);
}

/// A walk refusing many candidates still hands the rest out best first.
#[test]
fn a_walk_past_refused_candidates_continues_in_rank_order() {
    #[allow(clippy::cast_precision_loss)]
    let scored: Vec<(u64, f32)> = (0..200).map(|id| (id, id as f32)).collect();
    let mut ranked = Ranked::new(scored);
    let below_100: Vec<_> = ranked
        .by_ref()
        .filter(|&(id, _)| id < 100)
        .take(3)
        .collect();
    assert_eq!(below_100, [(99, 99.0), (98, 98.0), (97, 97.0)]);
    assert_eq!(ranked.next(), Some((96, 96.0)));
}

/// A walk whose admission refuses every candidate offers each one once, at
/// `O(log n)` comparisons apiece — never a fresh partition of the rest every
/// few refusals, which made such a walk quadratic. Counted in comparisons,
/// not time, so the bound holds on any machine.
#[test]
fn a_walk_refusing_every_candidate_makes_n_log_n_comparisons() {
    let n: u64 = 50_000;
    #[allow(clippy::cast_precision_loss)]
    let scored: Vec<(u64, f32)> = (0..n).map(|id| (id, (id % 9_973) as f32)).collect();
    let before = comparisons();
    let offered = Cell::new(0_u64);
    let taken = admitted(Ranked::new(scored), 10, |_| {
        offered.set(offered.get() + 1);
        false
    });
    let made = comparisons() - before;
    assert!(taken.is_empty());
    assert_eq!(offered.get(), n, "every candidate is offered once");
    // A heap pop sifts down at most `log2 n` levels, two comparisons each;
    // building the heap takes under `2n`.
    let log2_n = u64::from(n.next_power_of_two().ilog2());
    let bound = n * (2 * log2_n + 2);
    assert!(
        made <= bound,
        "{made} comparisons to refuse {n} candidates, more than n·(2·log2 n + 2) = {bound}"
    );
}

/// A NaN score — a sparse dot product can overflow into `inf - inf` — ranks
/// after every number whatever its sign, as `rank_scores` ranks it
/// everywhere else: under bare `total_cmp` a positive NaN ranked first.
#[test]
fn a_nan_score_ranks_last_whatever_its_sign() {
    let ranked = Ranked::new(vec![
        (1, f32::NAN),
        (2, 0.5),
        (3, -f32::NAN),
        (4, f32::INFINITY),
        (5, f32::NEG_INFINITY),
    ]);
    let order: Vec<u64> = ranked.map(|(id, _)| id).collect();
    assert_eq!(order, [4, 2, 5, 1, 3]);
}

/// The score order is `rank_scores`'s, whatever the magnitudes and signs:
/// the integer key orders every pair of scores as the comparator does, and
/// `-0.0` ties with `0.0`, leaving the order to the ids.
#[test]
fn the_rank_order_is_the_comparator_order() {
    let scores = [
        f32::NEG_INFINITY,
        f32::MIN,
        -1.5,
        -f32::MIN_POSITIVE,
        -0.0,
        0.0,
        f32::MIN_POSITIVE,
        1.5,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
        -f32::NAN,
    ];
    for &a in &scores {
        for &b in &scores {
            let by_key = super::rank_key(b, 0).cmp(&super::rank_key(a, 0));
            let by_comparator = crate::distance::rank_scores(a, b, true);
            assert_eq!(by_key, by_comparator, "{a} vs {b}");
        }
    }
    let tied = Ranked::new(vec![(2, 0.0), (1, -0.0)]);
    assert_eq!(tied.map(|(id, _)| id).collect::<Vec<_>>(), [1, 2]);
}

/// A restart puts back every candidate refused and every one set aside,
/// and the walk hands them all out again, best first.
#[test]
fn a_restart_hands_out_every_candidate_again_the_set_aside_ones_included() {
    let mut ranked = Ranked::keeping(vec![(1, 0.5), (2, 2.0), (3, 1.0), (4, 3.0)], |id| id != 2);
    let best = ranked.pop().expect("test: 4");
    let next = ranked.pop().expect("test: 3");
    assert_eq!((best.id, next.id), (4, 3));
    ranked.refuse(best);
    ranked.refuse(next);
    ranked.restart();
    assert_eq!(
        ranked.collect::<Vec<_>>(),
        [(4, 3.0), (2, 2.0), (3, 1.0), (1, 0.5)]
    );
}

/// A write that changes a refused or set-aside candidate puts it back among
/// those left, in rank order; one it does not change stays refused.
#[test]
fn a_reconsidered_candidate_is_handed_out_again_in_rank_order() {
    let mut ranked = Ranked::keeping(vec![(1, 0.5), (2, 2.0), (3, 1.0), (4, 3.0)], |id| id != 2);
    let best = ranked.pop().expect("test: 4");
    ranked.refuse(best);
    ranked.reconsider(2);
    assert_eq!(ranked.collect::<Vec<_>>(), [(2, 2.0), (3, 1.0), (1, 0.5)]);
}
