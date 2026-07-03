mod support;

use crate::support::environment::EnvironmentProfile;
use crate::support::scenario::run_attach_flow;

#[test]
fn attach_flow_brings_remote_context_into_workable_local_root() {
    run_attach_flow(&EnvironmentProfile::fs());
}
