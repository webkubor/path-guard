//! PATH 扫描与副本解析。
//!
//! 这里回答的是三个不同的问题，工具们通常只答第一个：
//!   1. 装了什么          —— `npm ls -g` 之类，包管理器视角
//!   2. **敲下去跑的是哪个** —— PATH 顺序 + 首次命中
//!   3. **它被谁挡住了**    —— 同名的其它副本排在后面，平时完全无感
//!
//! 2026-09-01 的真实事故：opencode 更新程序报「成功升级到 1.18.25」，而 PATH 第 7 位
//! 有个软链指向三个月前用另一套 node 装的 1.17.8，第 21 位才是刚更新的那个。
//! 包管理器没说谎，更新程序也没说谎，但用户敲 `opencode` 跑的一直是旧的。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// PATH 上的一个可执行文件副本。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 命令名
    pub name: String,
    /// 在 PATH 里的位置（0 起），决定谁先被命中
    pub path_index: usize,
    /// 该副本自身的路径（可能是软链）
    pub link: PathBuf,
    /// 一路解析到底的真实路径；软链悬空时为 None
    pub real: Option<PathBuf>,
    /// 是软链吗
    pub is_symlink: bool,
}

impl Entry {
    /// 悬空 = 是软链但目标不存在。
    ///
    /// 这类最阴险：`ls` 看得见、`which` 找得到，执行才报 "no such file"。
    /// 删掉某个 runtime 之后满地都是，而 grep 扫不出来——软链目标只存在于文件系统元数据里。
    pub fn is_dangling(&self) -> bool {
        self.is_symlink && self.real.is_none()
    }
}

/// 一个命令名下的全部副本，按 PATH 顺序排列。
#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    /// 按 path_index 升序；第一个就是实际会被执行的那个
    pub entries: Vec<Entry>,
}

impl Command {
    /// 实际会被执行的副本 —— PATH 里第一个能跑的。
    ///
    /// 注意跳过悬空软链：shell 命中它会直接报错，而不是继续往后找。
    /// 所以「winner 是悬空的」本身就是一种故障，要单独报出来。
    pub fn winner(&self) -> Option<&Entry> {
        self.entries.first()
    }

    /// 被遮蔽的副本 —— 存在但永远轮不到。
    pub fn shadowed(&self) -> &[Entry] {
        if self.entries.len() > 1 {
            &self.entries[1..]
        } else {
            &[]
        }
    }

    /// 是否存在多个**不同实体**的副本。
    ///
    /// 只看路径个数会误报：同一个文件被多处软链引用是正常的（homebrew 的 bin 布局就这样）。
    /// 必须按 realpath 去重，剩下 >1 才是真的装了多份。
    pub fn has_distinct_copies(&self) -> bool {
        let mut reals: Vec<&Path> = self
            .entries
            .iter()
            .filter_map(|e| e.real.as_deref())
            .collect();
        reals.sort();
        reals.dedup();
        reals.len() > 1
    }
}

/// 读取 PATH 并按顺序去重（保留首次出现的位置）。
///
/// 去重是必要的：`typeset -U path` 之类的设置只在 shell 内生效，
/// 传给子进程的 PATH 里重复段很常见，不去重会把同一个副本数两遍。
pub fn path_dirs() -> Vec<PathBuf> {
    let raw = std::env::var_os("PATH").unwrap_or_default();
    let mut seen = Vec::new();
    for p in std::env::split_paths(&raw) {
        if p.as_os_str().is_empty() {
            continue;
        }
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    seen
}

/// 判断一个文件是否可执行（Unix 权限位）。
#[cfg(unix)]
fn is_executable(md: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_md: &fs::Metadata) -> bool {
    true
}

/// 扫描 PATH 上所有可执行文件，按命令名归组。
///
/// 对每个条目都做两件事：区分「是不是软链」和「解析到底是谁」。
/// 这两个信息缺一不可 —— 只有 realpath 会漏掉悬空的（realpath 失败），
/// 只看 symlink 又分不清多个软链是否指向同一实体。
pub fn scan() -> BTreeMap<String, Command> {
    let mut map: BTreeMap<String, Command> = BTreeMap::new();

    for (idx, dir) in path_dirs().iter().enumerate() {
        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            // PATH 里指向已删目录的段是常态（删掉 runtime 之后就会有），静默跳过
            Err(_) => continue,
        };

        for ent in rd.flatten() {
            let link = ent.path();
            let name = match ent.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };

            // symlink_metadata 不跟随软链 —— 这样才能判断「它本身是不是软链」
            let lmd = match fs::symlink_metadata(&link) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_symlink = lmd.file_type().is_symlink();

            // 目标存在时才解析 realpath；悬空软链在这里得到 None
            let real = fs::canonicalize(&link).ok();

            // 可执行判断要看**目标**的权限，不是软链自己的
            let runnable = match &real {
                Some(r) => fs::metadata(r).map(|m| m.is_file() && is_executable(&m)).unwrap_or(false),
                // 悬空软链留下来 —— 它正是我们要报告的故障之一
                None => is_symlink,
            };
            if !runnable {
                continue;
            }

            map.entry(name.clone())
                .or_insert_with(|| Command {
                    name: name.clone(),
                    entries: Vec::new(),
                })
                .entries
                .push(Entry {
                    name,
                    path_index: idx,
                    link,
                    real,
                    is_symlink,
                });
        }
    }

    // 同一目录内不会有重名，所以按 path_index 排序即等价于「shell 的查找顺序」
    for cmd in map.values_mut() {
        cmd.entries.sort_by_key(|e| e.path_index);
    }
    map
}

/// 只扫一个命令，省去全量遍历。
pub fn scan_one(name: &str) -> Option<Command> {
    let mut entries = Vec::new();

    for (idx, dir) in path_dirs().iter().enumerate() {
        let link = dir.join(name);
        let lmd = match fs::symlink_metadata(&link) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_symlink = lmd.file_type().is_symlink();
        let real = fs::canonicalize(&link).ok();
        let runnable = match &real {
            Some(r) => fs::metadata(r).map(|m| m.is_file() && is_executable(&m)).unwrap_or(false),
            None => is_symlink,
        };
        if !runnable {
            continue;
        }
        entries.push(Entry {
            name: name.to_string(),
            path_index: idx,
            link,
            real,
            is_symlink,
        });
    }

    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|e| e.path_index);
    Some(Command {
        name: name.to_string(),
        entries,
    })
}
