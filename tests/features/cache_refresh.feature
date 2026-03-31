# language: zh-CN
功能: 缓存刷新
  作为 Section 用户
  我需要在外部修改数据后手动刷新缓存
  以便确保读取到最新数据

  背景:
    假如 Section 数据目录是干净的
    假如 本地测试目录 "/tmp/section-test-cache" 包含以下测试文件
    假如 已添加数据源 "cached" 使用 provider "fs" 选项 "root=/tmp/section-test-cache"

  场景: 读取后外部修改再刷新
    当 我执行 "section cat cached/hello.txt"
    那么 输出应该等于 "Hello Section"
    假如 外部直接修改文件 "/tmp/section-test-cache/hello.txt" 内容为 "updated"
    当 我执行 "section refresh cached/"
    那么 命令应该成功
    当 我执行 "section cat cached/hello.txt"
    那么 输出应该等于 "updated"
