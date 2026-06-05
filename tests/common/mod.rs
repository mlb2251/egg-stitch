//! Shared helpers for the snapshot (bless/check) test suites.
#![allow(dead_code)]

use serde_json::Value;

/// Recursively sort every JSON object's keys alphabetically, in place.
///
/// `serde_json::Value` backs objects with a `BTreeMap` (already sorted) unless
/// the `preserve_order` feature is enabled anywhere in the dependency graph, in
/// which case it's an insertion-ordered `IndexMap`. Sorting explicitly here
/// makes blessed fixtures alphabetical regardless of that feature, so snapshot
/// diffs stay minimal and order-stable rather than tracking struct field order.
pub fn sort_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, child) in entries.iter_mut() {
                sort_keys(child);
            }
            *map = entries.into_iter().collect();
        }
        Value::Array(items) => items.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

/// Returns a clone of `value` with all object keys recursively sorted.
pub fn sorted(value: &Value) -> Value {
    let mut v = value.clone();
    sort_keys(&mut v);
    v
}
