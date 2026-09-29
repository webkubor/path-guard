# path-guard

> [English](README.en.md) | 中文


查出你敲的命令**实际跑的是哪一个副本**、被谁遮蔽、以及更新到底生效没有。

## 它治什么病

2026-09-01，OpenCode 弹窗告诉我：

```
Update Complete
Successfully updated to OpenCode v1.18.25. Please restart the application.
```

重启，还是提示有新版本。再更新，再重启，还是。**更新程序说成功了，版本号纹丝不动。**

真相是机器上有四套 node 全局包，PATH 第 7 位有个软链指向三个月前用另一套 node 装的 1.17.8，而更新装进了第 21 位那套。

**没有任何一方在说谎**：npm 诚实报告「1.18.25 已安装」，更新程序看到 npm 退出码 0，shell 忠实执行 PATH 里第一个匹配。三个组件各自正确，合起来是错的。

顺着这条线查下去还牵出：`lark-cli` 版本倒挂（跑的 1.0.39、闲置的 1.0.69、npm 上 1.0.92）、106 个指向已删目录的悬空软链、12G 的僵尸副本。

现有工具都在回答**「我装了什么」**（`npm ls -g`、`brew list`、`pip list`）。没有工具回答**「我跑的是什么、它被谁挡着、装的和跑的对得上吗」**。

## 用法

```bash
path-guard doctor              # 一次性体检
path-guard which <cmd>         # 深度 which：命中谁、被谁遮蔽、各副本版本
path-guard dupes [--versions]  # 列出所有多副本命令
path-guard dangling            # 悬空软链（ls 看得见、执行才报错）
```

### `which` —— 最常用的一个

```
$ path-guard which opencode
命令 opencode

▶ 实际执行  ~/.local/share/mise/installs/node/24.15.0/bin/opencode
              → ~/.local/share/mise/installs/node/24.15.0/lib/node_modules/opencode-ai/bin/opencode.exe
   运行时版本 1.18.25
   所属包     opencode-ai@1.18.25

✅ 没有其它副本，不存在遮蔽
```

事故当天，同一条命令会显示 `运行时版本 1.17.8` 而 `所属包 opencode-ai@1.18.25` —— 一眼看出装的和跑的不是一个东西。

## 三条设计判断

**一、「多个路径」不等于「多份副本」。**
同一个文件被多处软链引用是正常布局（homebrew 的 bin 就这样）。必须按 `realpath` 去重，剩下 >1 才是真的装了多份。只数路径个数会满屏误报。

**二、运行时版本和声明版本要分开取，且都要取。**
- 运行时：真的执行 `<cmd> --version`，这是用户实际得到的
- 声明：从所属 `package.json` 读，这是包管理器认为的

两者分叉正是「更新没生效」的特征。只看任何一边都发现不了。

**三、包名不是身份。**
迁移时我按名字 `npm i -g facet`，装到的是 npm 上同名的另一个包（"Configuration mixin for constructors"），而我要的那个从来没发布过、是本地 `npm link` 的。所以 `declared_package` 连 name 一起返回，不只是版本号。

## 悬空软链为什么单独列一类

```
❌ ~/.local/bin/arkcli
   → ~/.nvm/versions/node/v24.15.0/bin/arkcli     # 这个目录已经删了
```

这类最阴险：`ls` 看得见、`which` 找得到、tab 补全能补出来，**执行才报 "no such file"**。

而且 `grep -r "nvm"` 扫不出来 —— 软链目标只存在于文件系统元数据里，不在任何文件内容中。只有 `find -type l` 逐个 `realpath` 才能发现。删掉某个 runtime 之后，这类残骸会散落在 PATH、应用数据目录、各种 `node_modules` 里。

## 装

```bash
cargo install --path .
# 或
cargo build --release && cp target/release/path-guard ~/.local/bin/
```

## 已知限制

- 版本探测靠执行 `--version`，不认这个 flag 的 CLI 拿不到运行时版本（会回退到只显示声明版本）
- 只扫 PATH。应用自己数据目录里的软链（如 `~/.dsh/profiles/node_modules`）目前不覆盖 —— 那次 DSH 的 106 个悬空链就在 PATH 之外
- 声明版本只认 `package.json`，pipx / brew / cargo 装的包还没接


## 安装到全局

```bash
cargo build --release
cp target/release/path-guard ~/.local/bin/     # 确保 ~/.local/bin 在 PATH 里
```

或 `cargo install --path .`（装到 `~/.cargo/bin`）。

装完直接用：

```bash
path-guard doctor
```

## `pinned` —— 唯一一条预防性检查

其余子命令都是事后诊断，只有它是防患于未然的。

```
$ path-guard pinned
PATH 里钉死了版本号的路径：2 条
⚠️  ~/Library/Python/3.14/bin
    版本段 3.14
⚠️  ~/.nvm/versions/node/v24.15.0/bin
    版本段 v24.15.0
    改用   nvm 没有版本无关入口，建议改用 mise/volta 之类提供 shims 的管理器
```

加它的理由是一次亲身踩坑：迁移到 mise 之后，我把 5 个 LaunchAgent 和两个脚本里的
`~/.nvm/versions/node/v24.15.0/bin` 改成了 `~/.local/share/mise/installs/node/24.15.0/bin`，
**自以为修好了，其实是用新工具重复了旧工具的错误设计** —— 路径里带版本号，把
「工具在哪」和「工具是哪个版本」焊死在一起。node 升到 25 的那天，这些会同时失效，
而且是延迟爆炸：下次那个服务重启时才报错，那时候根本想不起来跟今天的迁移有关。

正确写法是用版本管理器的**版本无关入口**：mise 用 `shims/`、pyenv/rbenv/asdf 同理、
homebrew 用 `opt/<formula>/bin` 而不是 `Cellar/<version>/`。

## License

MIT
