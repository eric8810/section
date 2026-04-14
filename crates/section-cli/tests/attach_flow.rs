mod support;

use crate::support::assert_success;
use crate::support::check::{
    assert_compare_ready, assert_local_content, assert_remote_content, assert_under_budget,
    SMALL_FLOW_BUDGET,
};
use crate::support::fixture::Fixture;
use std::time::Instant;

#[test]
fn attach_flow_brings_remote_context_into_workable_local_root() {
    let fixture = Fixture::fs_with_actors(1);
    fixture.seed_remote("docs/readme.txt", "remote-v1");
    fixture.seed_remote("tasks/todo.txt", "ship-tests");

    let actor = fixture.actor(0);
    actor.connect_fs(fixture.remote_root());

    let start = Instant::now();
    let sync = actor.sync();
    let elapsed = start.elapsed();
    assert_success(&sync, "source sync");
    assert_under_budget("attach sync", elapsed, SMALL_FLOW_BUDGET);

    assert_local_content(actor, "docs/readme.txt", "remote-v1");
    assert_local_content(actor, "tasks/todo.txt", "ship-tests");
    assert_remote_content(&fixture, "docs/readme.txt", "remote-v1");

    let compare = actor.compare("docs/readme.txt");
    assert_compare_ready(&compare);
}
