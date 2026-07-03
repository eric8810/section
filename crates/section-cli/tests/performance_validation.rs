mod support;

use crate::support::assert_success;
use crate::support::check::{assert_under_budget, SMALL_FLOW_BUDGET};
use crate::support::environment::EnvironmentProfile;
use crate::support::fixture::Fixture;
use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Serialize)]
struct PerformanceResult {
    file_count: usize,
    attach_ms: u128,
    noop_sync_ms: u128,
    incremental_sync_ms: u128,
}

#[test]
#[ignore = "performance validation is run separately from mainline tests"]
fn performance_validation_reports_attach_noop_and_incremental_sync_across_file_counts() {
    let counts = [10usize, 200, 1000];
    let mut results = Vec::new();

    for count in counts {
        let fixture = Fixture::for_environment(&EnvironmentProfile::fs(), 1);
        for index in 0..count {
            fixture.seed_remote(&format!("tree/file-{index:04}.txt"), "seed");
        }

        let actor = fixture.actor(0);
        actor.connect(fixture.source());

        let attach_start = Instant::now();
        let attach = actor.sync();
        let attach_elapsed = attach_start.elapsed();
        assert_success(&attach, "attach sync");
        assert_under_budget("attach sync", attach_elapsed, SMALL_FLOW_BUDGET * 6);

        let noop_start = Instant::now();
        let noop = actor.sync_json();
        let noop_elapsed = noop_start.elapsed();
        assert_eq!(noop["pulled"], 0);
        assert_eq!(noop["pushed"], 0);
        assert_under_budget("noop sync", noop_elapsed, SMALL_FLOW_BUDGET * 6);

        actor.write_local("tree/file-0000.txt", "changed");
        let incremental_start = Instant::now();
        let incremental = actor.sync();
        let incremental_elapsed = incremental_start.elapsed();
        assert_success(&incremental, "incremental sync");
        assert_under_budget(
            "incremental sync",
            incremental_elapsed,
            SMALL_FLOW_BUDGET * 6,
        );

        results.push(PerformanceResult {
            file_count: count,
            attach_ms: attach_elapsed.as_millis(),
            noop_sync_ms: noop_elapsed.as_millis(),
            incremental_sync_ms: incremental_elapsed.as_millis(),
        });
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&results).expect("performance json")
    );
}

#[test]
#[ignore = "performance validation is run separately from mainline tests"]
fn performance_validation_reports_conflict_detection_latency() {
    let fixture = Fixture::for_environment(&EnvironmentProfile::fs(), 2);
    fixture.seed_remote("docs/readme.txt", "base-v1");

    let actor_a = fixture.actor(0);
    let actor_b = fixture.actor(1);
    actor_a.connect(fixture.source());
    actor_b.connect(fixture.source());
    assert_success(&actor_a.sync(), "actor A initial sync");
    assert_success(&actor_b.sync(), "actor B initial sync");

    actor_a.write_local("docs/readme.txt", "edited-by-a");
    actor_b.write_local("docs/readme.txt", "edited-by-b");

    assert_success(&actor_a.sync(), "actor A first sync");
    let conflict_start = Instant::now();
    let conflict = actor_b.sync();
    let conflict_elapsed = conflict_start.elapsed();
    assert_success(&conflict, "actor B conflict sync");
    assert_under_budget(
        "conflict detection",
        conflict_elapsed,
        SMALL_FLOW_BUDGET * 3,
    );

    println!(
        "{}",
        serde_json::json!({
            "flow": "concurrent-conflict",
            "elapsed_ms": conflict_elapsed.as_millis(),
            "environment": "fs-cli",
        })
    );
}
