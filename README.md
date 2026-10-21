# unsafe-scan

一个零依赖的 Rust 命令行工具，用来统计和定位 Rust 项目中的 `unsafe` 代码。

A zero-dependency CLI that counts and locates `unsafe` code in Rust projects.

## 功能

- 递归扫描目录下所有 `.rs` 文件（自动跳过 `target/` 和隐藏目录）
- 按类型统计：`unsafe` 块、`unsafe fn`、`unsafe impl`、`unsafe trait`、`unsafe extern` 块
- 先剔除注释、字符串和字符字面量，文档或字符串里提到的 `unsafe` 不会被误计
- 支持表格、逐条列表和 JSON 三种输出
- `--deny` 模式：发现 `unsafe` 即返回退出码 1，可直接用于 CI

## 安装

```bash
git clone <本仓库地址>
cd unsafe-scan
cargo install --path .
```

## 使用

```bash
unsafe-scan                 # 扫描当前目录
unsafe-scan path/to/crate   # 扫描指定目录或单个 .rs 文件
unsafe-scan --list          # 列出每一处 unsafe 的位置
unsafe-scan --json          # 输出 JSON
unsafe-scan --deny          # 有 unsafe 时以退出码 1 结束
```

示例输出：

```
FILE           block     fn   impl  trait extern  total
./src/main.rs      1      1      0      0      0      2

./src/main.rs:4  unsafe block
./src/main.rs:6  unsafe fn

Scanned 1 files (6 lines): 2 unsafe item(s) in 1 file(s) [block=1, fn=1, impl=0, trait=0, extern=0]
```

## 实现说明

工具基于轻量的词法扫描，而非完整的语法分析：先用状态机把注释（含嵌套块注释）、普通/原始字符串、字符字面量替换为空格并保留换行，再根据 `unsafe` 关键字后面的 token 判断类型。`#[unsafe(no_mangle)]` 这类 2024 edition 的 unsafe 属性不计入统计。

宏展开生成的 `unsafe` 代码不会被统计。

## 测试

```bash
cargo test
```

## 许可证

[MIT](LICENSE)
