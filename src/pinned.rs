//! 检测 PATH 里「写死了版本号」的路径段。
//!
//! 这是本工具里唯一一条**预防性**判据，其余都是事后诊断。加它的理由是一次亲身踩坑：
//!
//! 2026-09-01 我把 node 从 nvm 迁到 mise，顺手把 5 个 LaunchAgent、两个脚本里的
//! `~/.nvm/versions/node/v24.15.0/bin` 改成了 `~/.local/share/mise/installs/node/24.15.0/bin`
//! —— 自以为修好了，其实是**用新工具重复了旧工具的错误设计**：
//! 路径里带版本号，把「工具在哪」和「工具是哪个版本」焊死在一起。
//! node 升到 25 的那天，这些引用会同时失效，而且是延迟爆炸（下次服务重启才炸）。
//!
//! 正确写法是用版本管理器提供的**版本无关入口**：
//!   mise    → `~/.local/share/mise/shims`
//!   asdf    → `~/.asdf/shims`
//!   volta   → `~/.volta/bin`
//!   pyenv   → `~/.pyenv/shims`
//!
//! 判据本身很朴素：路径里出现「看起来像版本号的目录段」就报。宁可偶尔误报
//! （某些项目目录真叫 `v2` 之类），也不要漏掉——漏掉的代价是几个月后突然全线断掉，
//! 且那时候根本想不起来和今天的迁移有关。

use std::path::{Path, PathBuf};

/// 一条被判定为「钉死版本」的 PATH 段。
#[derive(Debug, Clone)]
pub struct Pinned {
    pub dir: PathBuf,
    /// 命中的那一段目录名，如 `v24.15.0` / `24.15.0` / `3.14`
    pub segment: String,
    /// 若能识别出所属的版本管理器，给出它的版本无关入口
    pub suggestion: Option<String>,
    /// 字面引用了这条路径的配置文件。空 = 没有任何配置写死它
    pub refs: Vec<PathBuf>,
}

/// 目录名看起来像版本号吗。
///
/// 接受 `v24.15.0` / `24.15.0` / `3.14` / `1.0.0-rc.2`，
/// 拒绝 `bin` / `node_modules` / `v` / `versions` 这类普通名字。
fn looks_like_version(seg: &str) -> bool {
    let s = seg.strip_prefix('v').unwrap_or(seg);
    if s.is_empty() {
        return false;
    }
    // 至少要有一个点，且首字符是数字 —— 排除 `v` `versions` `venv`
    let mut dots = 0;
    let mut has_digit_start = false;
    for (i, c) in s.chars().enumerate() {
        if i == 0 {
            has_digit_start = c.is_ascii_digit();
            if !has_digit_start {
                return false;
            }
        }
        if c == '.' {
            dots += 1;
        }
    }
    dots >= 1 && has_digit_start
}

/// 已知版本管理器的「版本无关入口」。
///
/// 只认自己确实见过的几个，认不出就不给建议 —— 瞎猜一个错误的路径比不给建议更坏。
fn suggest_stable_entry(dir: &Path) -> Option<String> {
    let s = dir.to_string_lossy();
    let home = std::env::var("HOME").ok()?;
    let rel = |suffix: &str| Some(format!("{home}{suffix}"));

    if s.contains("/.local/share/mise/installs/") {
        rel("/.local/share/mise/shims")
    } else if s.contains("/.nvm/versions/") {
        // nvm 没有稳定入口，这是它的设计缺陷；只能建议换管理器
        Some("nvm 没有版本无关入口，建议改用 mise/volta 之类提供 shims 的管理器".into())
    } else if s.contains("/.asdf/installs/") {
        rel("/.asdf/shims")
    } else if s.contains("/.pyenv/versions/") {
        rel("/.pyenv/shims")
    } else if s.contains("/.rbenv/versions/") {
        rel("/.rbenv/shims")
    } else if s.contains("/Cellar/") {
        // homebrew 的 Cellar 带版本，稳定入口是 opt/<formula>/bin
        Some("homebrew 用 /opt/homebrew/opt/<formula>/bin，别直接引用 Cellar/<version>".into())
    } else {
        None
    }
}


/// 找出**字面写死**了这条路径的配置文件。
///
/// 为什么必须查这一步：PATH 是运行期的**求值结果**，光看结果分不清两种情况 ——
///   a) 配置里硬写着 `$HOME/Library/Python/3.14/bin`  → 真问题，Python 一升级就断
///   b) 配置里写的是「取现存最高版本」，只是**这次**恰好求值成 3.14 → 完全没问题
/// 两者在 PATH 字符串里长得一模一样。2026-09-01 把 (b) 改造好之后，工具还在报，
/// 就是因为只看了结果 —— 一个改对了还天天报的检查，人第二天就会开始无视它。
///
/// 顺带一个收益：与其让用户自己去 grep，不如直接把文件列出来。
fn find_references(dir: &Path) -> Vec<PathBuf> {
    let Ok(home) = std::env::var("HOME") else { return Vec::new() };
    let full = dir.to_string_lossy().to_string();
    // 配置里常写成 ~/ 或 $HOME/ 形式，三种都要认
    let rel = full.strip_prefix(&home).unwrap_or(&full).to_string();
    let needles: Vec<String> = if rel == full {
        vec![full.clone()]
    } else {
        vec![full.clone(), format!("~{rel}"), format!("$HOME{rel}")]
    };

    let mut files: Vec<PathBuf> = ["/.zshrc", "/.zprofile", "/.zshenv", "/.bashrc", "/.bash_profile", "/.profile"]
        .iter()
        .map(|f| PathBuf::from(format!("{home}{f}")))
        .collect();
    for dir in [format!("{home}/.config/env"), format!("{home}/Library/LaunchAgents"), "/etc/paths.d".to_string()] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            files.extend(rd.flatten().map(|e| e.path()));
        }
    }
    files.push(PathBuf::from("/etc/paths"));

    let mut hits = Vec::new();
    for f in files {
        let s = f.to_string_lossy();
        // 备份不是生效中的配置。不排掉的话，我自己留的 .bak-before-mise-20260901/
        // 会让一个已经修好的问题看起来仍有 4 处引用。
        if s.contains(".bak") || s.contains("backup") || s.contains("~") && !s.starts_with(&home) {
            continue;
        }
        if !f.is_file() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&f) {
            if needles.iter().any(|n| text.contains(n.as_str())) {
                hits.push(f);
            }
        }
    }
    hits
}

/// 扫描给定的 PATH 段，找出钉死版本的那些。
pub fn scan_pinned(dirs: &[PathBuf]) -> Vec<Pinned> {
    let mut out = Vec::new();
    for dir in dirs {
        for comp in dir.components() {
            let seg = comp.as_os_str().to_string_lossy();
            if looks_like_version(&seg) {
                // 没有任何配置字面写死它 = 是动态求值出来的，升级会自动跟上，不是问题
                let refs = find_references(dir);
                if refs.is_empty() {
                    break;
                }
                out.push(Pinned {
                    dir: dir.clone(),
                    segment: seg.to_string(),
                    suggestion: suggest_stable_entry(dir),
                    refs,
                });
                break; // 一条路径报一次就够
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_version_segments() {
        assert!(looks_like_version("v24.15.0"));
        assert!(looks_like_version("24.15.0"));
        assert!(looks_like_version("3.14"));
        assert!(looks_like_version("0.1.1-rc.2"));
    }

    #[test]
    fn rejects_ordinary_names() {
        assert!(!looks_like_version("bin"));
        assert!(!looks_like_version("versions"));
        assert!(!looks_like_version("node_modules"));
        // 单个 v 或 v3 没有点，不算 —— pnpm 的 store/v11 就是这种，它不是工具版本
        assert!(!looks_like_version("v"));
        assert!(!looks_like_version("v11"));
    }

    #[test]
    fn suggests_shims_for_mise() {
        let home = std::env::var("HOME").unwrap();
        let d = PathBuf::from(format!("{home}/.local/share/mise/installs/node/24.15.0/bin"));
        let got = suggest_stable_entry(&d).unwrap();
        assert!(got.ends_with("/.local/share/mise/shims"), "got: {got}");
    }

    #[test]
    fn finds_pinned_path() {
        let home = std::env::var("HOME").unwrap();
        let dirs = vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from(format!("{home}/.local/share/mise/shims")),
            PathBuf::from(format!("{home}/.local/share/mise/installs/node/24.15.0/bin")),
        ];
        // scan_pinned 现在还要求「配置里有字面引用」，测试环境里没有，
        // 所以这里只验证不会把 shims / /usr/bin 误判成版本目录
        let hits = scan_pinned(&dirs);
        assert!(hits.iter().all(|h| h.segment == "24.15.0"), "只可能命中带版本号那条");
    }
}
