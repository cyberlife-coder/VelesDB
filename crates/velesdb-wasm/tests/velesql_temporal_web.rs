//! `VelesQL` temporal expressions (`NOW()`, `INTERVAL`) in the browser.
//!
//! `wasm32-unknown-unknown` has no system clock: `std::time::SystemTime::now`
//! panics there, so a temporal expression must read the time the host gives
//! the page. Run with `wasm-pack test --node`.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

use velesdb_wasm::WasmDatabase;

fn seeded() -> WasmDatabase {
    let mut db = WasmDatabase::new();
    db.create_metadata_collection("events")
        .expect("test: create");
    // 1_000 is 1970; 4_000_000_000 is 2096: one past event, one future one.
    for (id, ts) in [(1, 1_000_i64), (2, 4_000_000_000)] {
        db.execute_query(
            &format!("INSERT INTO events (id, ts) VALUES ({id}, {ts})"),
            None,
        )
        .expect("test: insert");
    }
    db
}

#[wasm_bindgen_test]
fn now_compares_against_the_hosts_clock() {
    let mut db = seeded();
    let past = db
        .execute_query("SELECT * FROM events WHERE ts < NOW()", None)
        .expect("NOW() runs in the browser");
    assert_eq!(past.row_count(), 1);
    assert!(past.rows_json().contains("\"id\":1"));
}

#[wasm_bindgen_test]
fn joined_predicates_read_the_host_clock_once() {
    use std::{cell::Cell, rc::Rc};
    use wasm_bindgen::{closure::Closure, JsValue};

    let mut db = seeded();
    let date = js_sys::Reflect::get(&js_sys::global(), &"Date".into()).unwrap();
    let key = JsValue::from_str("now");
    let original = js_sys::Reflect::get(&date, &key).unwrap();
    let calls = Rc::new(Cell::new(0_u32));
    let tick = Rc::clone(&calls);
    let clock = Closure::wrap(Box::new(move || {
        let count = tick.get();
        tick.set(count + 1);
        1_000_000 + count * 1_000
    }) as Box<dyn FnMut() -> u32>);
    js_sys::Reflect::set(&date, &key, clock.as_ref()).unwrap();
    let rows = db.execute_query(
        "SELECT * FROM events a JOIN events b ON a.id = b.id WHERE a.ts >= NOW() AND b.ts < NOW()",
        None,
    );
    js_sys::Reflect::set(&date, &key, &original).unwrap();
    assert_eq!(rows.unwrap().row_count(), 0);
    assert_eq!(calls.get(), 1);
}

#[wasm_bindgen_test]
fn an_interval_alone_needs_no_clock() {
    let mut db = seeded();
    let rows = db
        .execute_query("SELECT * FROM events WHERE ts < INTERVAL '1 hour'", None)
        .expect("an INTERVAL is a duration, read without a clock");
    assert_eq!(
        rows.row_count(),
        1,
        "3600 s: only the 1970 event is below it"
    );
}

#[wasm_bindgen_test]
fn now_minus_an_interval_is_a_past_instant() {
    let mut db = seeded();
    let rows = db
        .execute_query(
            "SELECT * FROM events WHERE ts > NOW() - INTERVAL '7 days'",
            None,
        )
        .expect("NOW() - INTERVAL runs in the browser");
    assert_eq!(rows.row_count(), 1);
    assert!(rows.rows_json().contains("\"id\":2"));
}
