use crate::SectionWorld;
use cucumber::{then, when};
use std::fs;

/// 当 我执行 "{command}"
#[when(expr = "我执行 {string}")]
fn execute_command(world: &mut SectionWorld, command: String) {
    world.run_section_command(&command);
}

/// 那么 命令应该成功
#[then("命令应该成功")]
fn command_should_succeed(world: &mut SectionWorld) {
    assert!(
        world.last_success,
        "Expected command to succeed, but it failed.\nstdout: {}\nstderr: {}",
        world.last_stdout, world.last_stderr
    );
}

/// 那么 命令应该失败
#[then("命令应该失败")]
fn command_should_fail(world: &mut SectionWorld) {
    assert!(
        !world.last_success,
        "Expected command to fail, but it succeeded.\nstdout: {}",
        world.last_stdout
    );
}

/// 而且 输出应该包含 "{text}"
#[then(expr = "输出应该包含 {string}")]
fn output_should_contain(world: &mut SectionWorld, text: String) {
    assert!(
        world.last_stdout.contains(&text),
        "Expected stdout to contain '{text}'.\nActual stdout: {}",
        world.last_stdout
    );
}

/// 而且 输出不应该包含 "{text}"
#[then(expr = "输出不应该包含 {string}")]
fn output_should_not_contain(world: &mut SectionWorld, text: String) {
    assert!(
        !world.last_stdout.contains(&text),
        "Expected stdout NOT to contain '{text}'.\nActual stdout: {}",
        world.last_stdout
    );
}

/// 而且 输出应该等于 "{text}"
#[then(expr = "输出应该等于 {string}")]
fn output_should_equal(world: &mut SectionWorld, text: String) {
    let actual = world.last_stdout.trim_end();
    assert_eq!(
        actual, text,
        "Expected stdout to equal '{text}'.\nActual: '{actual}'"
    );
}

/// 而且 输出应该为空
#[then("输出应该为空")]
fn output_should_be_empty(world: &mut SectionWorld) {
    let trimmed = world.last_stdout.trim();
    assert!(
        trimmed.is_empty(),
        "Expected stdout to be empty.\nActual: '{}'",
        world.last_stdout
    );
}

/// 而且 本地文件 "{path}" 内容应该等于 "{content}"
#[then(expr = "本地文件 {string} 内容应该等于 {string}")]
fn local_file_should_equal(_world: &mut SectionWorld, path: String, content: String) {
    let actual = fs::read_to_string(&path).expect("failed to read local file");
    let actual = actual.trim_end();
    assert_eq!(
        actual, content,
        "Expected local file {path} to equal '{content}'.\nActual: '{actual}'"
    );
}
