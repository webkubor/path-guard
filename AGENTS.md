# AGENTS.md — AI 助手工作守则

## 这是什么

path-guard —— PATH 命令诊断 CLI（Rust）：定位实际执行的副本、版本遮蔽
（shadowing）、重复安装与悬空软链。回答的是「我敲的这个命令，到底跑的是哪一份」。

## 关键文件

- `src/`：Rust 源码（main + 诊断逻辑）。
- `Cargo.toml`：包定义；`cargo build` / `cargo test` 标准流程。

## 约定

- 诊断**只读不写**：永远不修改用户的 PATH 或文件，只报告与给建议。
- 跨 shell 兼容（zsh/bash/fish 的 PATH 语义差异）在测试里覆盖，别按单一 shell 假设。
- 双语 README（`README.md` / `README.en.md`）同步维护。
