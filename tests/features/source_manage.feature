# language: zh-CN
功能: 数据源管理
  作为 Section 用户
  我需要添加、查看和删除数据源
  以便连接不同的存储平台

  背景:
    假如 Section 数据目录是干净的

  场景: 添加本地文件系统数据源
    当 我执行 "section source add local-test --provider fs --opt root=/tmp/section-test"
    那么 命令应该成功
    而且 输出应该包含 "Source 'local-test' added"

  场景: 添加 S3 数据源
    当 我执行 "section source add my-s3 --provider s3 --opt bucket=test-bucket --opt region=us-east-1 --opt access_key_id=testkey --opt secret_access_key=testsecret"
    那么 命令应该成功
    而且 输出应该包含 "Source 'my-s3' added"

  场景: 列出数据源
    假如 已添加数据源 "local-test" 使用 provider "fs" 选项 "root=/tmp/section-test"
    假如 已添加数据源 "my-s3" 使用 provider "s3" 选项 "bucket=test-bucket"
    当 我执行 "section source list"
    那么 命令应该成功
    而且 输出应该包含 "local-test"
    而且 输出应该包含 "my-s3"

  场景: 删除数据源
    假如 已添加数据源 "to-remove" 使用 provider "fs" 选项 "root=/tmp/section-test"
    当 我执行 "section source remove to-remove"
    那么 命令应该成功
    而且 输出应该包含 "Source 'to-remove' removed"
    当 我执行 "section source list"
    那么 输出不应该包含 "to-remove"

  场景: 添加重名数据源时覆盖
    假如 已添加数据源 "dup" 使用 provider "fs" 选项 "root=/tmp/old"
    当 我执行 "section source add dup --provider fs --opt root=/tmp/new"
    那么 命令应该成功
    而且 输出应该包含 "Source 'dup' added"

  场景: 删除不存在的数据源
    当 我执行 "section source remove nonexistent"
    那么 命令应该成功
