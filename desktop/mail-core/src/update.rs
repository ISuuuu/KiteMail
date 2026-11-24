//! 发布清单（latest.json）检查、更新包下载与安装。
//!
//! Windows 版只有两种发布物：NSIS 安装包（`windows-x86_64`）与便携 zip
//! （`windows-x86_64-portable`）。清单由 `.github/workflows/windows.yml`
//! 在发布标签时生成，挂在最新 GitHub Release 上。

use std::collections::BTreeMap;
use std::io::Read;
#[cfg(windows)]
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::proxy;

/// 最新发布的更新清单（`releases/latest` 不会指向预发布版本）。
pub const MANIFEST_URL: &str =
    "https://github.com/ISuuuu/hylki/releases/latest/download/latest.json";

/// 安装版与便携版在清单里的平台键。
pub const TARGET_INSTALLER: &str = "windows-x86_64";
pub const TARGET_PORTABLE: &str = "windows-x86_64-portable";

const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RETRIES_WITHOUT_PROGRESS: u32 = 4;
const MAX_TOTAL_DOWNLOAD_ATTEMPTS: u32 = 12;
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;
const USER_AGENT: &str = concat!("Hylki/", env!("CARGO_PKG_VERSION"));

/// 清单里比当前版本更新的一次发布。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    /// 发布说明（来自发布页正文，可为空）。
    #[serde(default)]
    pub notes: String,
    /// 发布时间（ISO 8601，可为空）。
    #[serde(default)]
    pub pub_date: String,
    /// 更新包下载地址（HTTPS）。
    pub url: String,
    /// 更新包 SHA-256（小写十六进制）。
    pub sha256: String,
    /// 命中的平台键。
    pub target: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: String,
    #[serde(default)]
    platforms: BTreeMap<String, Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    url: String,
    #[serde(default)]
    sha256: String,
}

/// 当前运行形态对应的平台键：便携版目录里有 `portable.flag` 即视为便携版。
pub fn current_target() -> &'static str {
    if is_portable() {
        TARGET_PORTABLE
    } else {
        TARGET_INSTALLER
    }
}

/// 便携版目录里有 `portable.flag` 标记；文件缺失即视为安装版。
pub fn is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("portable.flag").is_file()))
        .unwrap_or(false)
}

/// 拉取默认清单并与 `current_version` 比较；有更新时返回对应平台的更新信息。
pub fn check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    check_manifest(MANIFEST_URL, current_version, current_target())
}

/// 拉取指定清单地址（便于测试与私有镜像）。
pub fn check_manifest(
    manifest_url: &str,
    current_version: &str,
    target: &str,
) -> Result<Option<UpdateInfo>, String> {
    if !manifest_url.starts_with("https://") {
        return Err("更新清单地址必须使用 HTTPS".into());
    }
    let (builder, proxy_label) = proxy::agent_builder(manifest_url);
    let agent = builder
        .https_only(true)
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build();
    let body = agent
        .get(manifest_url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format_proxy_error("无法获取更新清单", proxy_label.as_deref(), e))?
        .into_string()
        .map_err(|e| format_proxy_error("读取更新清单失败", proxy_label.as_deref(), e))?;
    // 兼容带 UTF-8 BOM 的清单（某些 Windows 工具默认会写入）。
    let manifest: Manifest = serde_json::from_str(body.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("更新清单无效: {e}"))?;
    select_update(manifest, current_version, target)
}

fn select_update(
    manifest: Manifest,
    current_version: &str,
    target: &str,
) -> Result<Option<UpdateInfo>, String> {
    if !is_newer(&manifest.version, current_version)? {
        return Ok(None);
    }
    let artifact = manifest
        .platforms
        .get(target)
        .ok_or_else(|| format!("更新清单没有 {target} 的构建"))?;
    if !artifact.url.starts_with("https://") || artifact.sha256.trim().is_empty() {
        return Err("更新包地址或校验值无效".into());
    }
    Ok(Some(UpdateInfo {
        version: manifest.version,
        current_version: current_version.to_string(),
        notes: manifest.notes,
        pub_date: manifest.pub_date,
        url: artifact.url.clone(),
        sha256: artifact.sha256.trim().to_ascii_lowercase(),
        target: target.to_string(),
    }))
}

/// 远端版本是否比当前版本新（预发布版本按 semver 规则比较）。
fn is_newer(remote: &str, current: &str) -> Result<bool, String> {
    let parse = |value: &str| {
        semver::Version::parse(value.trim().trim_start_matches('v'))
            .map_err(|e| format!("版本号无效（{value}）: {e}"))
    };
    let remote = parse(remote)?;
    let current = parse(current)?;
    Ok((!current.pre.is_empty() || remote.pre.is_empty()) && remote > current)
}

/// 下载更新包并在返回前校验 SHA-256；传输中断或遇临时网络错误时自动使用 HTTP Range 续传。
pub fn download(info: &UpdateInfo) -> Result<Vec<u8>, String> {
    if !info.url.starts_with("https://") {
        return Err("更新包地址必须使用 HTTPS".into());
    }
    download_with(
        &info.url,
        &info.sha256,
        true,
        MAX_PACKAGE_BYTES,
        proxy::agent_builder,
        std::thread::sleep,
    )
}

enum BodyReadError {
    TooLarge,
    Io(std::io::Error),
}

fn download_with<F, S>(
    url: &str,
    expected_sha256: &str,
    https_only: bool,
    max_bytes: u64,
    mut make_agent: F,
    mut sleep: S,
) -> Result<Vec<u8>, String>
where
    F: FnMut(&str) -> (ureq::AgentBuilder, Option<String>),
    S: FnMut(Duration),
{
    let mut bytes = Vec::new();
    let mut expected_total: Option<u64> = None;
    let mut consecutive_failures: u32 = 0;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        // 只限制连接与单次读写：安装包有几十 MB，整体超时会让慢网络在传输
        // 中途被掐断，报成「连接方没有正确答复」的套接字超时。
        // 每次尝试都重新构建 Agent，避免复用失效连接，并同步刷新代理与预签名重定向链接。
        let (builder, proxy_label) = make_agent(url);
        let agent = builder
            .https_only(https_only)
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(30))
            .build();

        let mut request = agent
            .get(url)
            .set("User-Agent", USER_AGENT)
            // 显式要求未压缩原始字节，保证首轮下载与后续 Range 续传偏移一致。
            .set("Accept-Encoding", "identity");
        if !bytes.is_empty() {
            request = request.set("Range", &format!("bytes={}-", bytes.len()));
        }

        let can_retry = |failures: u32| {
            attempts < MAX_TOTAL_DOWNLOAD_ATTEMPTS && failures <= MAX_RETRIES_WITHOUT_PROGRESS
        };

        let response = match request.call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(416, _)) if !bytes.is_empty() => {
                if expected_total == Some(bytes.len() as u64)
                    && verify_sha256(expected_sha256, &bytes).is_ok()
                {
                    return Ok(bytes);
                }
                bytes.clear();
                expected_total = None;
                consecutive_failures += 1;
                if can_retry(consecutive_failures) {
                    sleep(retry_backoff(consecutive_failures));
                    continue;
                }
                return Err(format_proxy_error(
                    "下载更新包失败",
                    proxy_label.as_deref(),
                    "服务器拒绝续传区间（HTTP 416）",
                ));
            }
            Err(err) => {
                consecutive_failures += 1;
                if is_retryable_ureq_error(&err) && can_retry(consecutive_failures) {
                    sleep(retry_backoff(consecutive_failures));
                    continue;
                }
                return Err(format_proxy_error(
                    "下载更新包失败",
                    proxy_label.as_deref(),
                    err,
                ));
            }
        };

        match response.status() {
            206 if !bytes.is_empty() => {
                let valid_range = response
                    .header("Content-Range")
                    .and_then(parse_content_range)
                    .filter(|(start, _, _)| *start == bytes.len() as u64);
                let Some((_, end, total)) = valid_range else {
                    bytes.clear();
                    expected_total = None;
                    consecutive_failures += 1;
                    if can_retry(consecutive_failures) {
                        sleep(retry_backoff(consecutive_failures));
                        continue;
                    }
                    return Err(format_proxy_error(
                        "读取更新包失败",
                        proxy_label.as_deref(),
                        "续传响应缺少有效的 Content-Range",
                    ));
                };
                if let Some(total) = total {
                    if total > max_bytes {
                        return Err("更新包超出大小限制".into());
                    }
                    if let Some(prev_total) = expected_total {
                        if prev_total != total {
                            bytes.clear();
                            expected_total = None;
                            consecutive_failures += 1;
                            if can_retry(consecutive_failures) {
                                sleep(retry_backoff(consecutive_failures));
                                continue;
                            }
                            return Err(format_proxy_error(
                                "下载更新包失败",
                                proxy_label.as_deref(),
                                "远端更新包总大小在续传过程中发生变化",
                            ));
                        }
                    } else {
                        expected_total = Some(total);
                    }
                } else if end.saturating_add(1) > max_bytes {
                    return Err("更新包超出大小限制".into());
                }
            }
            200 => {
                bytes.clear();
                let content_length = response
                    .header("Content-Length")
                    .and_then(|value| value.trim().parse::<u64>().ok());
                if let Some(len) = content_length {
                    if len > max_bytes {
                        return Err("更新包超出大小限制".into());
                    }
                }
                expected_total = content_length;
            }
            status => {
                return Err(format_proxy_error(
                    "下载更新包失败",
                    proxy_label.as_deref(),
                    format!("意外的 HTTP 状态码 {status}"),
                ));
            }
        }

        let before = bytes.len();
        match read_body_bounded(&mut response.into_reader(), &mut bytes, max_bytes) {
            Err(BodyReadError::TooLarge) => return Err("更新包超出大小限制".into()),
            Err(BodyReadError::Io(io_err)) => {
                if bytes.len() > before {
                    consecutive_failures = 0;
                } else {
                    consecutive_failures += 1;
                }
                if can_retry(consecutive_failures) {
                    sleep(retry_backoff(consecutive_failures));
                    continue;
                }
                return Err(format_proxy_error(
                    "读取更新包失败",
                    proxy_label.as_deref(),
                    io_err,
                ));
            }
            Ok(()) => match expected_total {
                Some(total) if bytes.len() as u64 == total => break,
                Some(total) if (bytes.len() as u64) < total => {
                    if bytes.len() > before {
                        consecutive_failures = 0;
                    } else {
                        consecutive_failures += 1;
                    }
                    if can_retry(consecutive_failures) {
                        sleep(retry_backoff(consecutive_failures));
                        continue;
                    }
                    return Err(format_proxy_error(
                        "读取更新包失败",
                        proxy_label.as_deref(),
                        format!("响应在 {}/{total} 字节处提前结束", bytes.len()),
                    ));
                }
                Some(_) => {
                    bytes.clear();
                    expected_total = None;
                    consecutive_failures += 1;
                    if can_retry(consecutive_failures) {
                        sleep(retry_backoff(consecutive_failures));
                        continue;
                    }
                    return Err(format_proxy_error(
                        "读取更新包失败",
                        proxy_label.as_deref(),
                        "实际接收字节数超出预期总长度",
                    ));
                }
                None => break,
            },
        }
    }

    verify_sha256(expected_sha256, &bytes)?;
    Ok(bytes)
}

fn read_body_bounded(
    reader: &mut dyn Read,
    bytes: &mut Vec<u8>,
    max_bytes: u64,
) -> Result<(), BodyReadError> {
    let mut buf = [0u8; DOWNLOAD_CHUNK_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if (bytes.len() as u64).saturating_add(n as u64) > max_bytes {
                    return Err(BodyReadError::TooLarge);
                }
                bytes.extend_from_slice(&buf[..n]);
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(BodyReadError::Io(err)),
        }
    }
}

fn parse_content_range(header: &str) -> Option<(u64, u64, Option<u64>)> {
    let (unit, spec) = header.trim().split_once(char::is_whitespace)?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range_part, total_part) = spec.trim().split_once('/')?;
    let (start_str, end_str) = range_part.trim().split_once('-')?;
    let start: u64 = start_str.trim().parse().ok()?;
    let end: u64 = end_str.trim().parse().ok()?;
    if start > end {
        return None;
    }
    let total = match total_part.trim() {
        "*" => None,
        raw => Some(raw.parse::<u64>().ok()?),
    };
    if let Some(total) = total {
        if end >= total {
            return None;
        }
    }
    Some((start, end, total))
}

fn is_retryable_ureq_error(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::Status(code, _) => matches!(code, 408 | 429 | 500 | 502 | 503 | 504),
        ureq::Error::Transport(transport) => matches!(
            transport.kind(),
            ureq::ErrorKind::ConnectionFailed
                | ureq::ErrorKind::Io
                | ureq::ErrorKind::Dns
                | ureq::ErrorKind::ProxyConnect
        ),
    }
}

fn retry_backoff(consecutive_failures: u32) -> Duration {
    match consecutive_failures {
        0 | 1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        3 => Duration::from_secs(4),
        _ => Duration::from_secs(5),
    }
}

fn format_proxy_error(
    prefix: &str,
    proxy_label: Option<&str>,
    err: impl std::fmt::Display,
) -> String {
    match proxy_label {
        Some(proxy) => format!("{prefix}（代理 {proxy}）: {err}"),
        None => format!("{prefix}: {err}"),
    }
}

fn verify_sha256(expected: &str, bytes: &[u8]) -> Result<(), String> {
    let digest = Sha256::digest(bytes);
    let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != expected.trim().to_ascii_lowercase() {
        return Err("更新包校验失败（SHA-256 不匹配）".into());
    }
    Ok(())
}

/// 安装已下载的更新包：安装版静默运行安装程序，便携版由后台脚本替换 exe。
/// 返回后调用方应立即退出，把运行位置让给新版本。
pub fn install(info: &UpdateInfo, bytes: &[u8]) -> Result<(), String> {
    if info.target.ends_with("-portable") {
        install_portable(bytes)
    } else {
        install_installer(bytes)
    }
}

#[cfg(windows)]
fn unique_update_dir() -> Result<std::path::PathBuf, String> {
    for attempt in 0..100 {
        let path = std::env::temp_dir().join(format!(
            "hylki-update-{}-{}-{attempt}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("创建更新目录失败: {e}")),
        }
    }
    Err("创建更新目录失败".into())
}

#[cfg(windows)]
fn install_installer(bytes: &[u8]) -> Result<(), String> {
    use std::ffi::OsString;
    use std::os::windows::process::CommandExt;

    if !bytes.starts_with(b"MZ") {
        return Err("更新包不是有效的安装程序".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序: {e}"))?;
    let install_dir = exe.parent().ok_or("无法定位安装目录")?;
    let mut directory_arg = OsString::from("/D=");
    directory_arg.push(install_dir);
    let path = unique_update_dir()?.join("setup.exe");
    std::fs::write(&path, bytes).map_err(|e| format!("写入安装程序失败: {e}"))?;
    std::process::Command::new(&path)
        .args(["/S", "/R", &format!("/PID={}", std::process::id())])
        // NSIS 要求 /D 参数位于末尾，带空格的路径也不加引号。
        .raw_arg(directory_arg)
        .spawn()
        .map_err(|e| format!("启动安装程序失败: {e}"))?;
    Ok(())
}

#[cfg(windows)]
fn install_portable(bytes: &[u8]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !bytes.starts_with(b"PK\x03\x04") {
        return Err("更新包不是有效的便携 zip".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序: {e}"))?;
    if !exe
        .parent()
        .is_some_and(|dir| dir.join("portable.flag").is_file())
    {
        return Err("当前不是便携版运行环境".into());
    }
    let zip = unique_update_dir()?.join("portable.zip");
    std::fs::write(&zip, bytes).map_err(|e| format!("写入便携更新包失败: {e}"))?;
    let script = portable_script(std::process::id(), &zip, &exe);
    let encoded = utf16le_base64(&script);
    std::process::Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .spawn()
        .map_err(|e| format!("启动便携更新脚本失败: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn install_installer(_bytes: &[u8]) -> Result<(), String> {
    Err("更新安装仅支持 Windows".into())
}

#[cfg(not(windows))]
fn install_portable(_bytes: &[u8]) -> Result<(), String> {
    Err("更新安装仅支持 Windows".into())
}

/// 等待当前进程退出后解压覆盖 exe 并重新启动；失败时把原程序拉起来。
#[cfg(windows)]
fn portable_script(pid: u32, zip: &Path, exe: &Path) -> String {
    let quote = |path: &Path| path.display().to_string().replace('\'', "\'\'");
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $zip = '{zip}'; $exe = '{exe}'\n\
         $stage = Join-Path (Split-Path -LiteralPath $zip) 'extract'\n\
         $new = $exe + '.new-' + {pid}; $backup = $exe + '.old-' + {pid}; $exited = $false\n\
         try {{\n\
           $process = Get-Process -Id {pid} -ErrorAction SilentlyContinue\n\
           if ($process -and -not $process.WaitForExit(60000)) {{ exit 1 }}\n\
           $exited = $true\n\
           Expand-Archive -LiteralPath $zip -DestinationPath $stage\n\
           $src = Join-Path $stage 'hylki.exe'\n\
           if (-not (Test-Path -LiteralPath $src)) {{ $src = Join-Path $stage 'hylki-windows.exe' }}\n\
           Copy-Item -LiteralPath $src -Destination $new\n\
           [System.IO.File]::Replace($new, $exe, $backup)\n\
           Start-Process -FilePath $exe\n\
           Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue\n\
         }} catch {{\n\
           if (Test-Path -LiteralPath $backup) {{\n\
             if (Test-Path -LiteralPath $exe) {{ [System.IO.File]::Replace($backup, $exe, $new) }}\n\
             else {{ [System.IO.File]::Move($backup, $exe) }}\n\
           }}\n\
           if ($exited -and (Test-Path -LiteralPath $exe)) {{ Start-Process -FilePath $exe -ErrorAction SilentlyContinue }}\n\
           exit 1\n\
         }} finally {{\n\
           Remove-Item -LiteralPath $new -Force -ErrorAction SilentlyContinue\n\
           Remove-Item -LiteralPath (Split-Path -LiteralPath $zip) -Recurse -Force -ErrorAction SilentlyContinue\n\
         }}\n",
        zip = quote(zip),
        exe = quote(exe),
    )
}

/// PowerShell 的 `-EncodedCommand` 需要 UTF-16LE + Base64，避开命令行引号问题。
#[cfg(windows)]
fn utf16le_base64(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Manifest {
        serde_json::from_str(&format!(
            r#"{{
                "version": "{version}",
                "notes": "修复若干问题",
                "pub_date": "2026-11-23T00:00:00Z",
                "platforms": {{
                    "windows-x86_64": {{ "url": "https://example.com/setup.exe", "sha256": "AA" }},
                    "windows-x86_64-portable": {{ "url": "https://example.com/portable.zip", "sha256": "bb" }}
                }}
            }}"#
        ))
        .unwrap()
    }

    #[test]
    fn selects_artifact_for_running_target_only_when_newer() {
        let update = select_update(manifest("1.50.0"), "1.49.0", TARGET_INSTALLER)
            .unwrap()
            .unwrap();
        assert_eq!(update.version, "1.50.0");
        assert_eq!(update.url, "https://example.com/setup.exe");
        assert_eq!(update.sha256, "aa");

        let portable = select_update(manifest("1.50.0"), "1.49.0", TARGET_PORTABLE)
            .unwrap()
            .unwrap();
        assert_eq!(portable.target, TARGET_PORTABLE);

        assert!(
            select_update(manifest("1.49.0"), "1.49.0", TARGET_INSTALLER)
                .unwrap()
                .is_none()
        );
        assert!(
            select_update(manifest("2.0.0-beta.1"), "1.49.0", TARGET_INSTALLER)
                .unwrap()
                .is_none()
        );
        assert!(
            select_update(manifest("2.0.0-beta.2"), "2.0.0-beta.1", TARGET_INSTALLER)
                .unwrap()
                .is_some()
        );
        assert!(
            select_update(manifest("1.49.0-beta.1"), "1.49.0", TARGET_INSTALLER)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_manifest_without_matching_target() {
        let error = select_update(manifest("2.0.0"), "1.0.0", "windows-aarch64").unwrap_err();
        assert!(error.contains("windows-aarch64"));
    }

    #[test]
    fn verifies_sha256_of_downloaded_package() {
        let bytes = b"payload";
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(verify_sha256(&digest, bytes).is_ok());
        assert!(verify_sha256(&digest.to_ascii_uppercase(), bytes).is_ok());
        assert!(verify_sha256(&digest, b"tampered").is_err());
    }

    #[test]
    #[cfg(windows)]
    fn portable_script_restarts_the_app_and_quotes_paths() {
        let script = portable_script(
            42,
            Path::new(r"C:\temp\it's.zip"),
            Path::new(r"C:\Hylki\hylki.exe"),
        );
        assert!(script.contains("$process.WaitForExit(60000)"));
        assert!(script.contains("it''s.zip"));
        assert!(script.contains("[System.IO.File]::Replace($new, $exe, $backup)"));
        assert!(script.contains("if ($process -and -not $process.WaitForExit(60000)) { exit 1 }"));
        assert!(script.contains("[System.IO.File]::Replace($backup, $exe, $new)"));
    }

    #[test]
    fn formats_errors_with_optional_proxy_context() {
        assert_eq!(
            format_proxy_error("无法获取更新清单", None, "timeout"),
            "无法获取更新清单: timeout"
        );
        assert_eq!(
            format_proxy_error("下载更新包失败", Some("http://127.0.0.1:7897"), "reset"),
            "下载更新包失败（代理 http://127.0.0.1:7897）: reset"
        );
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn spawn_mock_server<F>(
        max_requests: usize,
        handler: F,
    ) -> (String, std::thread::JoinHandle<Vec<String>>)
    where
        F: Fn(usize, &str, &mut std::net::TcpStream) + Send + 'static,
    {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut recorded = Vec::new();
            for index in 0..max_requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 1024];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                }
                let req = String::from_utf8_lossy(&raw).into_owned();
                recorded.push(req.clone());
                handler(index, &req, &mut stream);
                let _ = stream.flush();
            }
            recorded
        });
        (format!("http://{addr}/setup.exe"), handle)
    }

    #[test]
    fn parses_valid_and_invalid_content_range_headers() {
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some((100, 199, Some(200)))
        );
        assert_eq!(parse_content_range("BYTES 50-99/*"), Some((50, 99, None)));
        assert!(parse_content_range("bytes */200").is_none());
        assert!(parse_content_range("bytes 20-10/30").is_none());
        assert!(parse_content_range("items 0-9/10").is_none());
        assert!(parse_content_range("bytes 0-10/10").is_none());
    }

    #[test]
    fn resumes_interrupted_download_with_range_header() {
        use std::io::Write;
        let payload: Vec<u8> = (0..256).map(|i| (i % 251) as u8).collect();
        let expected_hash = sha256_hex(&payload);
        let payload_clone = payload.clone();

        let (url, server) = spawn_mock_server(2, move |attempt, req, stream| {
            if attempt == 0 {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload_clone.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&payload_clone[..100]).unwrap();
            } else {
                assert!(req.contains("Range: bytes=100-\r\n"));
                assert!(req.contains("Accept-Encoding: identity\r\n"));
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 100-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload_clone.len() - 1,
                    payload_clone.len(),
                    payload_clone.len() - 100
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&payload_clone[100..]).unwrap();
            }
        });

        let downloaded = download_with(
            &url,
            &expected_hash,
            false,
            1024,
            |_| (ureq::AgentBuilder::new(), None),
            |_| {},
        )
        .unwrap();
        assert_eq!(downloaded, payload);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[test]
    fn restarts_cleanly_when_server_ignores_range_with_200() {
        use std::io::Write;
        let payload: Vec<u8> = (0..200).map(|i| (i * 3 % 256) as u8).collect();
        let expected_hash = sha256_hex(&payload);
        let payload_clone = payload.clone();

        let (url, server) = spawn_mock_server(2, move |attempt, req, stream| {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload_clone.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            if attempt == 0 {
                stream.write_all(&payload_clone[..80]).unwrap();
            } else {
                assert!(req.contains("Range: bytes=80-\r\n"));
                stream.write_all(&payload_clone).unwrap();
            }
        });

        let downloaded = download_with(
            &url,
            &expected_hash,
            false,
            1024,
            |_| (ureq::AgentBuilder::new(), None),
            |_| {},
        )
        .unwrap();
        assert_eq!(downloaded, payload);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn retries_transient_503_and_stops_immediately_on_404() {
        use std::io::Write;
        let payload = b"ok-payload".to_vec();
        let expected_hash = sha256_hex(&payload);
        let payload_clone = payload.clone();

        let (url, server) = spawn_mock_server(2, move |attempt, _, stream| {
            if attempt == 0 {
                stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            } else {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload_clone.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&payload_clone).unwrap();
            }
        });

        let downloaded = download_with(
            &url,
            &expected_hash,
            false,
            1024,
            |_| (ureq::AgentBuilder::new(), None),
            |_| {},
        )
        .unwrap();
        assert_eq!(downloaded, payload);
        assert_eq!(server.join().unwrap().len(), 2);

        let (url_404, server_404) = spawn_mock_server(1, move |_, _, stream| {
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let err = download_with(
            &url_404,
            &expected_hash,
            false,
            1024,
            |_| (ureq::AgentBuilder::new(), None),
            |_| {},
        )
        .unwrap_err();
        assert!(err.contains("404"));
        assert_eq!(server_404.join().unwrap().len(), 1);
    }

    #[test]
    fn rejects_oversized_package_without_retrying() {
        use std::io::Write;
        let (url, server) = spawn_mock_server(1, move |_, _, stream| {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2048\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let err = download_with(
            &url,
            "00",
            false,
            1024,
            |_| (ureq::AgentBuilder::new(), None),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err, "更新包超出大小限制");
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn stops_after_retry_limit_on_persistent_failure() {
        use std::io::Write;
        let expected_attempts = (1 + MAX_RETRIES_WITHOUT_PROGRESS) as usize;
        let (url, server) = spawn_mock_server(expected_attempts, move |_, _, stream| {
            stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let err = download_with(
            &url,
            "00",
            false,
            1024,
            |_| {
                (
                    ureq::AgentBuilder::new(),
                    Some("http://127.0.0.1:7897".into()),
                )
            },
            |_| {},
        )
        .unwrap_err();
        assert!(err.contains("503"));
        assert!(err.contains("http://127.0.0.1:7897"));
        assert_eq!(server.join().unwrap().len(), expected_attempts);
    }
}
