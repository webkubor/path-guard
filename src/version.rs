//! 版本探测。
//!
//! 「装了 1.18.25」和「跑起来是 1.18.25」是两回事，本模块负责拿到后者。
//!
//! 两条独立的信息源，**必须都取，且分开呈现**：
//!   - 运行时版本：真的执行 `<cmd> --version`，这是用户实际得到的东西
//!   - 声明版本：从副本所属的 package.json 读，这是包管理器认为的东西
//!
//! 2026-09-01 的事故里，这两者会分叉：npm 说装了 1.18.25（package.json 是新的），
//! 但 postinstall 被拦、二进制没换，跑出来还是 1.17.8。只看任何一边都发现不了。

use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};
use std::time::Duration;

/// 执行 `<bin> --version` 取运行时版本。
///
/// 超时是必须的：有些 CLI 不认 `--version` 会进交互模式或等 stdin，
/// 直接挂死在这里会让整个体检卡住。stdin 接 null 也是同样的原因。
pub fn runtime_version(bin: &Path) -> Option<String> {
    for flag in ["--version", "-V", "version"] {
        let out = Proc::new(bin)
            .arg(flag)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()
            .and_then(|child| wait_timeout(child, Duration::from_secs(5)));

        if let Some(text) = out {
            if let Some(v) = extract_semver(&text) {
                return Some(v);
            }
        }
    }
    None
}

/// 简易超时等待。
///
/// 标准库没有带超时的 wait，这里用轮询 try_wait 实现——精度不重要，
/// 目的只是防止一个不听话的 CLI 把体检卡住。超时就 kill。
fn wait_timeout(mut child: std::process::Child, limit: Duration) -> Option<String> {
    use std::io::Read;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let mut buf = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut buf);
                }
                if buf.trim().is_empty() {
                    // 不少 CLI 把版本打到 stderr
                    if let Some(mut e) = child.stderr.take() {
                        let _ = e.read_to_string(&mut buf);
                    }
                }
                return Some(buf);
            }
            Ok(None) => {
                if start.elapsed() > limit {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(_) => return None,
        }
    }
}

/// 从任意输出里抠出第一个形如 `1.2.3` 的版本号。
///
/// 不用正则依赖：手写扫描，避免为这点小事拖进 regex crate。
/// 允许前缀 v、允许后缀 -rc.2 这类 pre-release（DSH 的版本就是 0.1.1-rc.2）。
pub fn extract_semver(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_digit() {
                    j += 1;
                } else if c == b'.' && j + 1 < bytes.len() && bytes[j + 1].is_ascii_digit() {
                    dots += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            // 恰好两个点才是 semver。写 `>= 2` 会把 IP 吃进来：
            // `127.0.0.1` 有三个点，宽松匹配会返回 "127.0.0.1"，
            // 而如果只取前三段又会静默给出 "127.0.0" —— 后者更糟，是错误答案而非无答案。
            // 所以四段及以上一律跳过，继续往后找。
            if dots == 2 {
                // 带上 pre-release / build 后缀：-rc.2、-beta.1、+build
                let mut end = j;
                if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
                    let mut k = end + 1;
                    while k < bytes.len()
                        && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'.' || bytes[k] == b'-')
                    {
                        k += 1;
                    }
                    end = k;
                }
                return Some(text[start..end].to_string());
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// 沿目录树上溯找 package.json，读出 name + version。
///
/// 为什么要往上找：npm 装的 CLI，bin 通常在 `<pkg>/bin/x.js` 或更深，
/// package.json 在包根。找到第一个带 name 字段的就停。
///
/// **name 一并返回，不只是 version** —— 因为「包名不是身份」：
/// 2026-09-01 我按名字重装 `facet`，装到的是 npm 上同名的另一个包
/// （"Configuration mixin for constructors"），而我要的那个从来没发布过、是本地 link 的。
/// 只比版本号发现不了这种，必须连 name 一起看。
pub fn declared_package(bin: &Path) -> Option<(String, String)> {
    let mut dir = bin.parent()?;
    for _ in 0..6 {
        let pj = dir.join("package.json");
        if pj.is_file() {
            if let Ok(text) = std::fs::read_to_string(&pj) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    let name = v.get("name").and_then(|x| x.as_str());
                    let ver = v.get("version").and_then(|x| x.as_str());
                    if let (Some(n), Some(ve)) = (name, ver) {
                        return Some((n.to_string(), ve.to_string()));
                    }
                }
            }
        }
        dir = dir.parent()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_semver() {
        assert_eq!(extract_semver("1.18.25"), Some("1.18.25".into()));
        assert_eq!(extract_semver("v24.15.0"), Some("24.15.0".into()));
    }

    #[test]
    fn parses_from_noisy_output() {
        assert_eq!(
            extract_semver("ripgrep 15.2.0\n-SIMD -AVX"),
            Some("15.2.0".into())
        );
        assert_eq!(
            extract_semver("mise 2026.8.16 macos-arm64"),
            Some("2026.8.16".into())
        );
    }

    #[test]
    fn keeps_prerelease_suffix() {
        // DSH 就是这种版本号，截断成 0.1.1 会让两份不同的包看起来一样
        assert_eq!(extract_semver("dsh 0.1.1-rc.2"), Some("0.1.1-rc.2".into()));
    }

    #[test]
    fn ignores_incomplete_versions() {
        // 两段的不算版本号
        assert_eq!(extract_semver("only 1.2 here"), None);
    }

    #[test]
    fn does_not_mistake_ip_for_version() {
        // 四段一律跳过。宽松匹配会返回 "127.0.0.1"，只取前三段又会给出 "127.0.0" ——
        // 后者是**错误答案**，比没有答案危险得多
        assert_eq!(extract_semver("listening on 127.0.0.1:3080"), None);
        // 但同一行里若真有版本号，仍要能找到
        assert_eq!(
            extract_semver("server 2.4.1 on 127.0.0.1:3080"),
            Some("2.4.1".into())
        );
    }
}

/// 通过 shim 拿版本是不可靠的 —— 垫片的 `--version` 打的是**管理器自己的版本**。
///
/// 2026-09-01 实测：`~/.local/share/mise/shims/bun --version` 返回 `2026.8.16`
/// （mise 的版本），而真实的 bun 是 1.4.0。这种输出比没有输出更坏：
/// 它看起来是个正常答案，排查时会把人带偏。
///
/// 所以对 shim 要先问管理器「你实际会执行哪个文件」，再对那个文件取版本。
pub fn resolve_shim_target(shim: &Path) -> Option<PathBuf> {
    let name = shim.file_name()?.to_str()?;
    // 逐个试，某个管理器不存在就跳过——这里用 `?` 会让第一个缺失的管理器
    // 直接终止整个查找，是刚写的时候犯的错
    for mgr in ["mise", "asdf", "rtx"] {
        let Some(bin) = which_manager(mgr) else { continue };
        let Ok(out) = Proc::new(&bin)
            .arg("which")
            .arg(name)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() && Path::new(&p).is_file() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

fn which_manager(name: &str) -> Option<PathBuf> {
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
