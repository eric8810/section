use crate::support::assert_success;
use crate::support::check::{
    assert_compare_conflict, assert_compare_ready, assert_local_content, assert_remote_content,
    assert_under_budget, SMALL_FLOW_BUDGET,
};
use crate::support::environment::EnvironmentProfile;
use crate::support::fixture::Fixture;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize)]
pub struct FlowMeasurement {
    pub environment: String,
    pub flow: String,
    pub elapsed_ms: u128,
}

pub fn run_attach_flow(profile: &EnvironmentProfile) -> FlowMeasurement {
    let fixture = Fixture::for_environment(profile, 1);
    fixture.seed_remote("docs/readme.txt", "remote-v1");
    fixture.seed_remote("tasks/todo.txt", "ship-tests");

    let actor = fixture.actor(0);
    actor.connect(fixture.source());

    let start = Instant::now();
    let sync = actor.sync();
    let elapsed = start.elapsed();
    assert_success(&sync, "source sync");
    assert_under_budget("attach sync", elapsed, SMALL_FLOW_BUDGET);

    assert_local_content(actor, "docs/readme.txt", "remote-v1");
    assert_local_content(actor, "tasks/todo.txt", "ship-tests");
    assert_remote_content(&fixture, "docs/readme.txt", "remote-v1");
    assert_compare_ready(&actor.compare("docs/readme.txt"));

    measurement(profile, "attach", elapsed)
}

pub fn run_handoff_flow(profile: &EnvironmentProfile) -> FlowMeasurement {
    let fixture = Fixture::for_environment(profile, 2);
    fixture.seed_remote("docs/readme.txt", "base-v1");

    let actor_a = fixture.actor(0);
    let actor_b = fixture.actor(1);
    actor_a.connect(fixture.source());
    actor_b.connect(fixture.source());

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

    let a_final_start = Instant::now();
    assert_success(&actor_a.sync(), "actor A final sync");
    let total_elapsed = a_sync_elapsed + b_sync_elapsed + a_final_start.elapsed();
    assert_under_budget("handoff flow", total_elapsed, SMALL_FLOW_BUDGET * 3);

    assert_local_content(actor_a, "docs/readme.txt", "edited-by-b");
    assert_compare_ready(&actor_a.compare("docs/readme.txt"));
    assert_compare_ready(&actor_b.compare("docs/readme.txt"));

    measurement(profile, "handoff", total_elapsed)
}

pub fn run_concurrent_flow(profile: &EnvironmentProfile) -> FlowMeasurement {
    let fixture = Fixture::for_environment(profile, 2);
    fixture.seed_remote("docs/readme.txt", "base-v1");

    let actor_a = fixture.actor(0);
    let actor_b = fixture.actor(1);
    actor_a.connect(fixture.source());
    actor_b.connect(fixture.source());

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
    assert_compare_conflict(&actor_b.compare("docs/readme.txt"));

    let events = actor_b.watch_once();
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "conflict_detected"),
        "expected conflict_detected event, got {events:?}"
    );

    measurement(profile, "concurrent", conflict_elapsed)
}

fn measurement(profile: &EnvironmentProfile, flow: &str, elapsed: Duration) -> FlowMeasurement {
    FlowMeasurement {
        environment: profile.name.clone(),
        flow: flow.to_string(),
        elapsed_ms: elapsed.as_millis(),
    }
}
