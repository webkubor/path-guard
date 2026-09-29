# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.1.0] - 2026-09-10

主发布：

- `which` / `dupes` / `dangling` / `pinned` / `doctor` 五个子命令
- 三个明确的设计判断：版本探测依赖 `--version`、PATH 外不扫、pipx/brew/cargo 装的包没接（已写在 README 已知限制段）

## 历史（按时间倒序）

- `f414299` — chore: 补 MIT LICENSE 文件
- `0d05d8d` — feat(pinned): 只报「配置里字面写死」的版本路径，不报动态求值结果
- `7c15b5e` — feat(doctor): `--json` 输出供 cs diag runtime 消费；`dangling` 排除系统拥有文件
- `2db0e00` — fix: shadowed 展示也要排除 shim 转发目标
- `2ca52df` — fix: shim 的版本探测要先解析真身，别把管理器版本当成程序版本
- `7d04ea0` — fix: 识别版本管理器的 shim，消除 mise 迁移后的多副本误报
- `bfc4172` — docs: License 段挪回文末
- `f777e53` — feat: path-guard 0.1.0 —— 查命令实际跑哪个副本、被谁遮蔽、更新是否生效

---

## 已知缺口

- **零自动化测试**（`cargo test` 当前不跑任何用例）—— 后续需补 integration test
- **依赖项版本探测受限**：`--version` 风格 + PATH 内 + shim-aware；pipx / brew / cargo install 的包未接
- 无英文 README（项目被 cs / hypit 等英文消费者引用）

完整 commit 列表见 `git log`。