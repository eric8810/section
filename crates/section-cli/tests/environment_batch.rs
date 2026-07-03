mod support;

use crate::support::environment::EnvironmentProfile;
use crate::support::scenario::{run_attach_flow, run_concurrent_flow, run_handoff_flow};

#[test]
fn core_scenarios_run_across_available_environment_profiles() {
    for profile in EnvironmentProfile::available() {
        run_attach_flow(&profile);
        run_handoff_flow(&profile);
        run_concurrent_flow(&profile);
    }
}
