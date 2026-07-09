pub mod agent;
pub mod commit;
pub mod file;
pub mod fs;
pub mod hooks;
pub mod init;
pub mod mount;
pub mod path;
pub mod source;
pub mod status;
pub mod watch;

pub(crate) fn print_agentfs_json_error(err: &anyhow::Error, command: &'static str) {
    if let Some(agentfs_error) = err.downcast_ref::<sectiond::AgentFsError>() {
        println!(
            "{}",
            serde_json::json!({ "error": agentfs_error.payload() })
        );
        return;
    }

    println!(
        "{}",
        serde_json::json!({
            "error": {
                "code": "operation_failed",
                "message": err.to_string(),
                "retryable": false,
                "details": {
                    "command": command,
                },
            }
        })
    );
}
