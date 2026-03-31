use crate::SectionWorld;
use cucumber::given;
use std::fs;
use std::io::Write;
use std::path::Path;

/// 假如 Section 数据目录是干净的
#[given("Section 数据目录是干净的")]
fn clean_data_dir(world: &mut SectionWorld) {
    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");

    // Write a minimal config pointing data_dir to our temp dir
    let config_content = format!(
        "data_dir = \"{}\"\nmount_point = \"/tmp/section-mount-test\"\n",
        tmp.path().display()
    );
    let config_path = tmp.path().join("config.toml");
    fs::write(&config_path, &config_content).expect("failed to write config");

    world.data_dir = Some(tmp);
}

/// Helper: create a directory with files.
fn setup_test_dir(base: &Path, files: &[(&str, &str)]) {
    let _ = fs::remove_dir_all(base);
    fs::create_dir_all(base).expect("failed to create test dir");
    for (path, content) in files {
        let file_path = base.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent dir");
        }
        let content = content.replace("\\n", "\n");
        fs::write(&file_path, &content).expect("failed to write test file");
    }
}

// --- file_read.feature background ---

#[given(expr = "本地测试目录 {string} 包含以下测试文件")]
fn create_read_test_dir(world: &mut SectionWorld, path: String) {
    let _ = world;
    let base = Path::new(&path);
    setup_test_dir(
        base,
        &[
            ("hello.txt", "Hello Section"),
            ("docs/readme.md", "# README"),
            ("docs/guide.md", "# Guide"),
            ("data/config.yaml", "key: value"),
        ],
    );
}

// --- file_copy.feature background ---

#[given(expr = "本地测试目录 {string} 包含拷贝源测试文件")]
fn create_copy_src_dir(world: &mut SectionWorld, path: String) {
    let _ = world;
    let base = Path::new(&path);
    setup_test_dir(
        base,
        &[
            ("report.pdf", "PDF_CONTENT"),
            ("data/a.csv", "col1,col2"),
        ],
    );
}

// --- file_exec.feature background ---

#[given(expr = "本地测试目录 {string} 包含可执行测试脚本")]
fn create_exec_test_dir(world: &mut SectionWorld, path: String) {
    let _ = world;
    let base = Path::new(&path);
    setup_test_dir(
        base,
        &[
            ("hello.sh", "#!/bin/bash\necho \"hello exec\""),
            ("args.sh", "#!/bin/bash\necho \"$1 $2\""),
            ("fail.sh", "#!/bin/bash\nexit 1"),
        ],
    );
    // Make scripts executable
    for name in &["hello.sh", "args.sh", "fail.sh"] {
        let p = base.join(name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755))
                .expect("failed to chmod script");
        }
    }
}

/// 假如 本地测试目录 "{path}" 是空的
#[given(expr = "本地测试目录 {string} 是空的")]
fn create_empty_test_dir(world: &mut SectionWorld, path: String) {
    let _ = world;
    let base = Path::new(&path);
    let _ = fs::remove_dir_all(base);
    fs::create_dir_all(base).expect("failed to create empty test dir");
}

/// 假如 已添加数据源 "{name}" 使用 provider "{provider}" 选项 "{opts}"
#[given(expr = "已添加数据源 {string} 使用 provider {string} 选项 {string}")]
fn add_source(world: &mut SectionWorld, name: String, provider: String, opts: String) {
    let mut args = format!("source add {name} --provider {provider}");
    for kv in opts.split_whitespace() {
        args.push_str(&format!(" --opt {kv}"));
    }
    world.run_section_command(&args);
    assert!(
        world.last_success,
        "Failed to add source '{name}': {}",
        world.last_stderr
    );
}

/// 假如 数据源 "{source}" 中 "{file_path}" 内容为 "{content}"
#[given(expr = "数据源 {string} 中 {string} 内容为 {string}")]
fn source_file_with_content(
    world: &mut SectionWorld,
    _source: String,
    file_path: String,
    content: String,
) {
    let _ = world;
    // In BDD tests, "store" source always maps to /tmp/section-test-write
    let base = std::path::Path::new("/tmp/section-test-write");
    let full_path = base.join(&file_path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&full_path, content.as_bytes()).unwrap();
}

/// 假如 外部直接修改文件 "{path}" 内容为 "{content}"
#[given(expr = "外部直接修改文件 {string} 内容为 {string}")]
fn external_modify_file(world: &mut SectionWorld, path: String, content: String) {
    let _ = world;
    let mut f = fs::File::create(&path).expect("failed to open file for external modify");
    f.write_all(content.as_bytes())
        .expect("failed to write external modification");
}
