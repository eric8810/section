# language: zh-CN
功能: 文件拷贝
  作为 Section 用户
  我需要在不同数据源之间拷贝文件
  以便在多个存储间迁移和备份数据

  背景:
    假如 Section 数据目录是干净的
    假如 本地测试目录 "/tmp/section-test-src" 包含拷贝源测试文件
    假如 本地测试目录 "/tmp/section-test-dst" 是空的
    假如 已添加数据源 "src" 使用 provider "fs" 选项 "root=/tmp/section-test-src"
    假如 已添加数据源 "dst" 使用 provider "fs" 选项 "root=/tmp/section-test-dst"

  场景: 跨 source 拷贝单个文件
    当 我执行 "section cp src/report.pdf dst/report.pdf"
    那么 命令应该成功
    当 我执行 "section cat dst/report.pdf"
    那么 输出应该等于 "PDF_CONTENT"

  场景: 跨 source 拷贝子目录中的文件
    当 我执行 "section cp src/data/a.csv dst/data/a.csv"
    那么 命令应该成功
    当 我执行 "section cat dst/data/a.csv"
    那么 输出应该等于 "col1,col2"

  场景: 拷贝不存在的文件
    当 我执行 "section cp src/no-file.txt dst/no-file.txt"
    那么 命令应该失败

  场景: 同一 source 内拷贝
    当 我执行 "section cp src/report.pdf src/report-backup.pdf"
    那么 命令应该成功
    当 我执行 "section cat src/report-backup.pdf"
    那么 输出应该等于 "PDF_CONTENT"

  场景: 从本地文件拷贝到 source
    假如 本地文件 "/tmp/section-test-local/local.txt" 内容为 "LOCAL_CONTENT"
    当 我执行 "section cp /tmp/section-test-local/local.txt dst/from-local.txt"
    那么 命令应该成功
    当 我执行 "section cat dst/from-local.txt"
    那么 输出应该等于 "LOCAL_CONTENT"

  场景: 从 source 拷贝到本地文件
    当 我执行 "section cp src/report.pdf /tmp/section-test-out/report.pdf"
    那么 命令应该成功
    而且 本地文件 "/tmp/section-test-out/report.pdf" 内容应该等于 "PDF_CONTENT"

  场景: 递归拷贝目录
    当 我执行 "section cp -r src/data/ dst/copied-data/"
    那么 命令应该成功
    当 我执行 "section cat dst/copied-data/a.csv"
    那么 输出应该等于 "col1,col2"
