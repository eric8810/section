mod support;

use crate::support::assert_success;
use crate::support::check::{
    assert_compare_ready, assert_local_content, assert_remote_content, assert_under_budget,
    SMALL_FLOW_BUDGET,
};
use crate::support::fixture::Fixture;
use std::time::Instant;

#[test]
fn handoff_flow_allows_two_participants_to_continue_the_same_context() {
    let fixture = Fixture::fs_with_actors(2);
    fixture.seed_remote("docs/readme.txt", "base-v1");

    let actor_a = fixture.actor(0);
    let actor_b = fixture.actor(1);
    actor_a.connect_fs(fixture.remote_root());
    actor_b.connect_fs(fixture.remote_root());

    assert_success(&actor_a.sync(), "actor A initial sync");
    assert_success(&actor_b.sync(), "actor B initial sync");

    actor_a.write_local("docs/readme.txt", "edited-by-a");
    let a_sync_start = Instant::now();
    let a_sync = actor_a.sync();
    let a_sync_elapsed = a_sync_start.elapsed();
    assert_success(&a_sync, "actor A handoff sync");
    assert_under_budget("actor A handoff sync", a_sync_elapsed, SMALL_FLOW_BUDGET);

    assert_remote_content(&fixture, "docs/readme.txt", "edited-by-a");

    let b_sync_start = Instant::now();
    let b_sync = actor_b.sync();
    let b_sync_elapsed = b_sync_start.elapsed();
    assert_success(&b_sync, "actor B receive sync");
    assert_under_budget("actor B receive sync", b_sync_elapsed, SMALL_FLOW_BUDGET);
    assert_local_content(actor_b, "docs/readme.txt", "edited-by-a");
    assert_compare_ready(&actor_b.compare("docs/readme.txt"));

    actor_b.write_local("docs/readme.txt", "edited-by-b");
    assert_success(&actor_b.sync(), "actor B return sync");
    assert_remote_content(&fixture, "docs/readme.txt", "edited-by-b");

    assert_success(&actor_a.sync(), "actor A final sync");
    assert_local_content(actor_a, "docs/readme.txt", "edited-by-b");
    assert_compare_ready(&actor_a.compare("docs/readme.txt"));
    assert_compare_ready(&actor_b.compare("docs/readme.txt"));
}
