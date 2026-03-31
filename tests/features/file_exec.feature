# language: zh-CN
功能: 远端文件执行
  作为 Section 用户
  我需要执行数据源中的脚本文件
  以便在本地运行远端存储的脚本

  背景:
    假如 Section 数据目录是干净的
    假如 本地测试目录 "/tmp/section-test-exec" 包含可执行测试脚本
    假如 已添加数据源 "scripts" 使用 provider "fs" 选项 "root=/tmp/section-test-exec"

  场景: 执行简单脚本
    当 我执行 "section exec scripts/hello.sh"
    那么 命令应该成功
    而且 输出应该包含 "hello exec"

  场景: 执行带参数的脚本
    当 我执行 "section exec scripts/args.sh -- foo bar"
    那么 命令应该成功
    而且 输出应该包含 "foo bar"

  场景: 执行失败的脚本返回非零退出码
    当 我执行 "section exec scripts/fail.sh"
    那么 命令应该失败

  场景: 执行不存在的脚本
    当 我执行 "section exec scripts/nope.sh"
    那么 命令应该失败
