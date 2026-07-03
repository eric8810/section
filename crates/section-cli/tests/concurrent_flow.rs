mod support;

use crate::support::environment::EnvironmentProfile;
use crate::support::scenario::run_concurrent_flow;

#[test]
fn concurrent_flow_surfaces_conflict_without_silent_overwrite() {
    run_concurrent_flow(&EnvironmentProfile::fs());
}
