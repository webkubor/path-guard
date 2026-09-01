//! path-guard —— 查出你敲的命令实际跑的是哪一个副本、被谁遮蔽、更新到底生效没有。
//!
//! 起因是 2026-09-01 的一次排查：OpenCode 反复提示「更新成功」但版本纹丝不动。
//! 挖到最后是机器上有四套 node 全局包，PATH 第 7 位的软链指向三个月前的副本，
//! 而更新装进了第 21 位那套。包管理器、更新程序、shell 全都没说谎，结果却是错的。
//!
//! 现有工具都在回答「我装了什么」，没有工具回答「我跑的是什么、它被谁挡着」。

mod pinned;
mod scan;
mod version;

use clap::{Parser, Subcommand};
use scan::{scan, scan_one, Command as Cmd};
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "path-guard",
    version,
    about = "查出命令实际跑的是哪个副本、被谁遮蔽、更新是否生效",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    cmd: Sub,
}

#[derive(Subcommand)]
enum Sub {
    /// 深度 which：实际命中谁、被谁遮蔽、各副本版本
    Which {
        /// 命令名
        name: String,
    },
    /// 列出 PATH 上存在多个不同实体副本的命令
    Dupes {
        /// 连同各副本的版本一起探测（会逐个执行 --version，慢）
        #[arg(long)]
        versions: bool,
    },
    /// 找出悬空软链（指向已删目标，ls 看得见、执行才报错）
    Dangling,
    /// 找出 PATH 里写死版本号的路径段（升级后会集体失效）
    Pinned,
    /// 一次性体检：多副本 + 悬空 + 钉死版本
    Doctor {
        /// 输出 JSON，供 cs diag runtime 等上游消费
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Sub::Which { name } => cmd_which(&name),
        Sub::Dupes { versions } => cmd_dupes(versions),
        Sub::Dangling => cmd_dangling(),
        Sub::Pinned => cmd_pinned(),
        Sub::Doctor { json } => cmd_doctor(json),
    }
}

fn short(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

fn cmd_which(name: &str) {
    let Some(cmd) = scan_one(name) else {
        println!("❌ PATH 上找不到 {name}");
        std::process::exit(1);
    };

    let winner = cmd.winner().unwrap();
    println!("命令 {}", cmd.name);
    println!();

    // 实际命中的那个
    print!("▶ 实际执行  {}", short(&winner.link));
    if winner.is_symlink {
        match &winner.real {
            Some(r) => print!("\n              → {}", short(r)),
            None => print!("\n              → 悬空！目标不存在"),
        }
    }
    println!();
    // shim 的 --version 打的是管理器自己的版本（实测 shims/bun 返回 mise 的 2026.8.16），
    // 必须先问管理器要到真身，再对真身取版本 —— 否则给出的是个看似正常的错误答案
    let probe: Option<std::path::PathBuf> = if scan::is_shim(winner) {
        match version::resolve_shim_target(&winner.link) {
            Some(t) => {
                println!("   shim 转发到 {}", short(&t));
                Some(t)
            }
            None => None,
        }
    } else {
        winner.real.clone()
    };
    if let Some(real) = &probe {
        if let Some(v) = version::runtime_version(real) {
            println!("   运行时版本 {v}");
        }
        if let Some((pkg, pv)) = version::declared_package(real) {
            println!("   所属包     {pkg}@{pv}");
        }
    }

    // 被遮蔽的
    let shadowed = cmd.shadowed();
    if shadowed.is_empty() {
        println!("\n✅ 没有其它副本，不存在遮蔽");
        return;
    }

    // winner 若是 shim，它转发到的那个目标也会出现在 shadowed 里——那是同一个东西的两种
    // 到达方式，不是「被遮蔽的另一份安装」。不排除的话，每个走 shim 的命令都会假报一条。
    let shim_target = if scan::is_shim(winner) {
        version::resolve_shim_target(&winner.link)
    } else {
        None
    };
    let real_shadowed: Vec<&scan::Entry> = shadowed
        .iter()
        .filter(|e| match &shim_target {
            // 它就是 shim 转发过去的那个（不论按链接路径还是解析后的真身比对）
            Some(t) => e.link != *t && e.real.as_deref() != Some(t.as_path()),
            None => true,
        })
        .collect();

    if real_shadowed.is_empty() {
        println!("\n✅ 没有其它副本，不存在遮蔽");
        return;
    }

    println!("\n被它遮蔽的（存在但永远轮不到）：");
    for e in real_shadowed {
        // 同一实体的多个软链无害，标出来避免误伤
        let same = match (&winner.real, &e.real) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let tag = if same {
            "（同一实体，无害）"
        } else if e.is_dangling() {
            "⚠️ 悬空"
        } else {
            "⚠️ 不同实体"
        };
        println!("   {} {}", short(&e.link), tag);
        if let Some(r) = &e.real {
            if !same {
                println!("      → {}", short(r));
                if let Some((pkg, pv)) = version::declared_package(r) {
                    println!("        {pkg}@{pv}");
                }
            }
        }
    }

    if cmd.has_distinct_copies() {
        println!("\n多个不同实体同名 —— 更新装进哪一个、你跑的是哪一个，值得核对。");
    }
}

fn cmd_dupes(with_versions: bool) {
    let all = scan();
    let dupes: Vec<&Cmd> = all.values().filter(|c| c.has_distinct_copies()).collect();

    if dupes.is_empty() {
        println!("✅ PATH 上没有多副本命令");
        return;
    }

    println!("存在多个不同实体的命令：{} 个\n", dupes.len());
    for cmd in dupes {
        println!("{}", cmd.name);
        for e in &cmd.entries {
            let marker = if e.path_index == cmd.entries[0].path_index {
                "▶"
            } else {
                " "
            };
            let mut line = format!("  {} [{}] {}", marker, e.path_index, short(&e.link));
            if e.is_dangling() {
                line.push_str("  ⚠️ 悬空");
            }
            println!("{line}");
            if with_versions {
                if let Some(r) = &e.real {
                    let rv = version::runtime_version(r);
                    let dp = version::declared_package(r);
                    match (rv, dp) {
                        (Some(v), Some((pkg, pv))) if v != pv => {
                            // 运行时与声明不一致 —— 正是 opencode 那次的特征
                            println!("       运行 {v} / 声明 {pkg}@{pv}  ⚠️ 不一致");
                        }
                        (Some(v), Some((pkg, pv))) => println!("       {v}（{pkg}@{pv}）"),
                        (Some(v), None) => println!("       {v}"),
                        (None, Some((pkg, pv))) => println!("       {pkg}@{pv}（取不到运行时版本）"),
                        (None, None) => {}
                    }
                }
            }
        }
        println!();
    }
}

fn cmd_dangling() {
    let all = scan();
    let mut found = 0;
    for cmd in all.values() {
        for e in &cmd.entries {
            if e.is_dangling() && !scan::is_system_owned(&e.link) {
                found += 1;
                println!("❌ {}", short(&e.link));
                if let Ok(t) = std::fs::read_link(&e.link) {
                    println!("   → {}", short(&t));
                }
            }
        }
    }
    if found == 0 {
        println!("✅ PATH 上没有悬空软链");
    } else {
        println!("\n共 {found} 个。这类 ls 看得见、which 找得到，执行才报错。");
    }
}

fn cmd_pinned() {
    let dirs = scan::path_dirs();
    let hits = pinned::scan_pinned(&dirs);
    if hits.is_empty() {
        println!("✅ PATH 里没有写死版本号的路径段");
        return;
    }
    println!("PATH 里钉死了版本号的路径：{} 条\n", hits.len());
    for h in &hits {
        println!("⚠️  {}", short(&h.dir));
        println!("    版本段 {}", h.segment);
        if let Some(s) = &h.suggestion {
            println!("    改用   {s}");
        }
        // 直接列出写死它的文件 —— 比让用户自己去 grep 快，也避免搜错关键词
        for r in &h.refs {
            println!("    写在   {}", short(r));
        }
    }
    println!("\n这类路径在工具升级后会集体失效，而且是延迟爆炸——下次那个服务/脚本被调用时才报错。");
}

fn cmd_doctor(as_json: bool) {
    let all = scan();
    let dupes: Vec<&Cmd> = all.values().filter(|c| c.has_distinct_copies()).collect();
    let dangling: Vec<(&Cmd, &scan::Entry)> = all
        .values()
        .flat_map(|c| c.entries.iter().map(move |e| (c, e)))
        .filter(|(_, e)| e.is_dangling() && !scan::is_system_owned(&e.link))
        .collect();

    if as_json {
        // 只输出「需要处理的」——上游（cs diag runtime）按 id 做「只推变化」的去重，
        // 所以 id 必须稳定：同一个问题每次跑出来要是同一个字符串。
        let mut items: Vec<serde_json::Value> = Vec::new();
        for (_, e) in &dangling {
            items.push(serde_json::json!({
                "id": format!("dangling/{}", e.link.display()),
                "kind": "dangling",
                "title": format!("悬空软链 {}", short(&e.link)),
                "detail": std::fs::read_link(&e.link).map(|t| format!("→ {}", short(&t))).unwrap_or_default(),
            }));
        }
        for h in pinned::scan_pinned(&scan::path_dirs()) {
            // detail 里带上「写在哪几个文件」，上游直接照抄就能定位。
            // 在 json! 外面算好：宏里塞不了块表达式。
            let mut detail = h.suggestion.clone().unwrap_or_default();
            for r in &h.refs {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(&format!("写在 {}", short(r)));
            }
            items.push(serde_json::json!({
                "id": format!("pinned/{}", h.dir.display()),
                "kind": "pinned",
                "title": format!("PATH 写死版本号 {}", short(&h.dir)),
                "detail": detail,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "findings": items })).unwrap());
        return;
    }

    println!("path-guard 体检");
    println!("  PATH 段数      {}", scan::path_dirs().len());
    println!("  可执行命令     {}", all.len());
    println!();

    if dangling.is_empty() {
        println!("  ✅ 悬空软链     无");
    } else {
        println!("  ⚠️  悬空软链     {} 个（执行才报错）", dangling.len());
        for (_, e) in dangling.iter().take(5) {
            println!("       {}", short(&e.link));
        }
        if dangling.len() > 5 {
            println!("       …… 还有 {} 个，看 `path-guard dangling`", dangling.len() - 5);
        }
    }

    if dupes.is_empty() {
        println!("  ✅ 多副本命令   无");
    } else {
        println!("  ⚠️  多副本命令   {} 个（更新可能装错地方）", dupes.len());
        for c in dupes.iter().take(8) {
            let n = c.entries.len();
            println!("       {} （{} 处）", c.name, n);
        }
        if dupes.len() > 8 {
            println!("       …… 还有 {} 个，看 `path-guard dupes`", dupes.len() - 8);
        }
    }

    let pins = pinned::scan_pinned(&scan::path_dirs());
    if pins.is_empty() {
        println!("  ✅ 钉死版本     无");
    } else {
        println!("  ⚠️  钉死版本     {} 条（工具升级后会集体失效）", pins.len());
        for h in pins.iter().take(4) {
            println!("       {}", short(&h.dir));
        }
        if pins.len() > 4 {
            println!("       …… 还有 {} 条，看 `path-guard pinned`", pins.len() - 4);
        }
    }

    println!();
    if dupes.is_empty() && dangling.is_empty() && pins.is_empty() {
        println!("一切正常。");
    } else {
        println!("逐个细查：path-guard which <命令名>");
    }
}
