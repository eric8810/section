# language: zh-CN
功能: 文件写入与删除
  作为 Section 用户
  我需要通过 CLI 写入和删除数据源中的文件
  以便管理存储中的数据

  背景:
    假如 Section 数据目录是干净的
    假如 本地测试目录 "/tmp/section-test-write" 是空的
    假如 已添加数据源 "store" 使用 provider "fs" 选项 "root=/tmp/section-test-write"

  场景: 通过管道写入文件
    当 我执行管道 "echo -n 'test content'" 到 "section write store/new-file.txt"
    那么 命令应该成功
    当 我执行 "section cat store/new-file.txt"
    那么 输出应该等于 "test content"

  场景: 覆盖已有文件
    假如 数据源 "store" 中 "exist.txt" 内容为 "old"
    当 我执行管道 "echo -n 'new'" 到 "section write store/exist.txt"
    那么 命令应该成功
    当 我执行 "section cat store/exist.txt"
    那么 输出应该等于 "new"

  场景: 删除单个文件
    假如 数据源 "store" 中 "to-delete.txt" 内容为 "bye"
    当 我执行 "section rm store/to-delete.txt"
    那么 命令应该成功
    当 我执行 "section cat store/to-delete.txt"
    那么 命令应该失败

  场景: 递归删除目录
    假如 数据源 "store" 中 "dir/a.txt" 内容为 "a"
    假如 数据源 "store" 中 "dir/b.txt" 内容为 "b"
    当 我执行 "section rm store/dir/ -r"
    那么 命令应该成功
    当 我执行 "section ls store/dir/"
    那么 输出应该为空
