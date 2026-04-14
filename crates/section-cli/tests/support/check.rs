use crate::support::actor::Actor;
use crate::support::fixture::Fixture;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

pub const SMALL_FLOW_BUDGET: Duration = Duration::from_secs(5);

pub fn assert_file_content(path: &Path, expected: &str) {
    let actual = std::fs::read_to_string(path).expect("read file");
    assert_eq!(
        actual,
        expected,
        "unexpected file content at {}",
        path.display()
    );
}

pub fn assert_local_content(actor: &Actor, relative_path: &str, expected: &str) {
    assert_eq!(
        actor.read_local(relative_path),
        expected,
        "unexpected local content for {relative_path}"
    );
}

pub fn assert_remote_content(fixture: &Fixture, relative_path: &str, expected: &str) {
    assert_eq!(
        fixture.read_remote(relative_path),
        expected,
        "unexpected remote content for {relative_path}"
    );
}

pub fn assert_compare_ready(compare: &Value) {
    assert_eq!(compare["state"], "ready");
    assert_eq!(compare["local_matches_current_remote"], true);
}

pub fn assert_compare_conflict(compare: &Value) {
    assert_eq!(compare["state"], "conflict");
    assert_eq!(compare["stale"], true);
}

pub fn assert_under_budget(label: &str, elapsed: Duration, budget: Duration) {
    assert!(
        elapsed <= budget,
        "{label} exceeded budget: {:?} > {:?}",
        elapsed,
        budget
    );
}
