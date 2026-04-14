mod support;

use crate::support::assert_success;
use crate::support::check::{
    assert_compare_conflict, assert_local_content, assert_remote_content, assert_under_budget,
    SMALL_FLOW_BUDGET,
};
use crate::support::fixture::Fixture;
use std::time::Instant;

#[test]
fn concurrent_flow_surfaces_conflict_without_silent_overwrite() {
    let fixture = Fixture::fs_with_actors(2);
    fixture.seed_remote("docs/readme.txt", "base-v1");

    let actor_a = fixture.actor(0);
    let actor_b = fixture.actor(1);
    actor_a.connect_fs(fixture.remote_root());
    actor_b.connect_fs(fixture.remote_root());

    assert_success(&actor_a.sync(), "actor A initial sync");
    assert_success(&actor_b.sync(), "actor B initial sync");

    actor_a.write_local("docs/readme.txt", "edited-by-a");
    actor_b.write_local("docs/readme.txt", "edited-by-b");

    assert_success(&actor_a.sync(), "actor A wins first sync");
    let conflict_start = Instant::now();
    let b_sync = actor_b.sync();
    let conflict_elapsed = conflict_start.elapsed();
    assert_success(&b_sync, "actor B conflict sync");
    assert_under_budget("conflict surfacing", conflict_elapsed, SMALL_FLOW_BUDGET);

    assert_remote_content(&fixture, "docs/readme.txt", "edited-by-a");
    assert_local_content(actor_b, "docs/readme.txt", "edited-by-b");

    let compare = actor_b.compare("docs/readme.txt");
    assert_compare_conflict(&compare);

    let events = actor_b.watch_once();
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "conflict_detected"),
        "expected conflict_detected event, got {events:?}"
    );
}
