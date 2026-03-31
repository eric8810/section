# language: zh-CN
功能: 文件读取
  作为 Section 用户
  我需要通过 CLI 浏览和读取数据源中的文件
  以便快速查看和获取数据

  背景:
    假如 Section 数据目录是干净的
    假如 本地测试目录 "/tmp/section-test-read" 包含以下测试文件
    假如 已添加数据源 "test" 使用 provider "fs" 选项 "root=/tmp/section-test-read"

  场景: 列出根目录下所有 source
    当 我执行 "section ls"
    那么 命令应该成功
    而且 输出应该包含 "test/"

  场景: 列出 source 根目录的文件
    当 我执行 "section ls test/"
    那么 命令应该成功
    而且 输出应该包含 "hello.txt"
    而且 输出应该包含 "docs/"
    而且 输出应该包含 "data/"

  场景: 列出子目录的文件
    当 我执行 "section ls test/docs/"
    那么 命令应该成功
    而且 输出应该包含 "readme.md"
    而且 输出应该包含 "guide.md"

  场景: 读取文件内容
    当 我执行 "section cat test/hello.txt"
    那么 命令应该成功
    而且 输出应该等于 "Hello Section"

  场景: 读取子目录中的文件
    当 我执行 "section cat test/docs/readme.md"
    那么 命令应该成功
    而且 输出应该等于 "# README"

  场景: 读取不存在的文件
    当 我执行 "section cat test/nonexistent.txt"
    那么 命令应该失败

  场景: 列出不存在的 source
    当 我执行 "section ls no-such-source/"
    那么 命令应该失败

  场景: 列出不存在的目录
    当 我执行 "section ls test/no-such-dir/"
    那么 命令应该成功
    而且 输出应该为空
