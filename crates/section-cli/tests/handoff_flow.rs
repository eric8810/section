mod support;

use crate::support::environment::EnvironmentProfile;
use crate::support::scenario::run_handoff_flow;

#[test]
fn handoff_flow_allows_two_participants_to_continue_the_same_context() {
    run_handoff_flow(&EnvironmentProfile::fs());
}
