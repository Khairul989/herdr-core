//! `hc update`: swap this hc for another build, keep the terminals running.

use super::*;

/// What a binary says it is (`hc 0.2.0`), if it is a working hc.
pub(super) fn version_of(path: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(path).arg("--version").stdin(std::process::Stdio::null()).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && v.starts_with("hc ")).then_some(v)
}

/// A custom build never takes a release (it would drop its local patches and
/// identity). When `source-update.env` beside config.toml names its checkout
/// (`HC_SOURCE_DIR`), that checkout's update-from-source.sh merges upstream,
/// then builds, signs, installs and restarts; otherwise it is refused.
pub(super) fn from_source(paths: &Paths) -> Result<Out, RpcError> {
    let config = paths.config.with_file_name("source-update.env");
    let dir = std::fs::read_to_string(&config).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix("HC_SOURCE_DIR=").map(|v| v.trim_matches(['"', '\'']).to_owned()))
    });
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return Err(RpcError::new(
            "refused",
            "ordinary updates would replace this custom build",
            format!(
                "merge upstream source, build, test and sign it, then run hc update --from with the signed local bundle; \
                 or set HC_SOURCE_DIR and HC_SOURCE_BRANCH in {} so hc update does that",
                config.display()
            ),
        ));
    };
    let script = std::path::Path::new(&dir).join("scripts/update-from-source.sh");
    let out = std::process::Command::new("sh")
        .arg(&script)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| {
            RpcError::new("refused", format!("cannot run {}: {e}", script.display()), "check HC_SOURCE_DIR")
        })?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        // The script's own verdict when it gave one, else the end of the build output.
        let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let why = match lines.iter().rev().find_map(|l| l.strip_prefix("update-from-source: ")) {
            Some(verdict) => verdict.to_owned(),
            None => lines[lines.len().saturating_sub(8)..].join("\n"),
        };
        return Err(RpcError::new(
            "refused",
            format!("source update failed: {why}"),
            format!("fix it in {dir}, then run hc update again"),
        ));
    }
    let now = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_owned();
    let human = format!("{now}, built from {dir}");
    Ok(Out { json: json!({"now": now, "updated": true, "source": dir}), human })
}

/// A release name for `--version`: `0.1.4` (it goes into a URL).
pub(super) fn release_version(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.split('.').collect();
    let number = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    if parts.len() == 3 && parts.iter().all(number) {
        Ok(s.into())
    } else {
        Err(format!("expected a release like 0.1.4, got {s:?}"))
    }
}

/// `hc 0.1.9` is older than `hc 0.2.0`; anything unparsed is not older.
pub(super) fn older(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> {
        v.strip_prefix("hc ")?.split(['.', '-']).take(3).map(|p| p.parse().ok()).collect()
    };
    matches!((parts(a), parts(b)), (Some(a), Some(b)) if a < b)
}

/// The installed binary can be current while a core started before it was
/// replaced is still running. Do not start a core just to check its version.
pub(super) fn up_to_date(paths: &Paths, release: &str, human: String) -> Result<Out, RpcError> {
    let this = format!("hc {}", env!("CARGO_PKG_VERSION"));
    let mut out = Out { json: json!({"release": release, "now": this, "updated": false}), human };
    if let Some(stream) = client::connect_existing(paths) {
        let status = client::call(&stream, "server.status", json!({}))?;
        if older(&format!("hc {}", status["version"].as_str().unwrap_or_default()), &this) {
            client::call(&stream, "server.restart", json!({}))?;
            wait_closed(&stream);
            let status = call(paths, "server.status", json!({}))?;
            out.json["restarted"] = true.into();
            out.json["pid"] = status["pid"].clone();
            out.human.push_str(&format!("; older core restarted (pid {}), terminals kept", status["pid"]));
        }
    }
    Ok(out)
}

/// Swaps the binary this hc runs from for `from`, then restarts the core
/// the planned way: every terminal keeps running. One swap path, for people
/// and for apps that bundle hc.
pub(super) fn update(paths: &Paths, from: &std::path::Path, force: bool) -> Result<Out, RpcError> {
    let source = crate::platform::update_source(from);
    let from = source.as_path();
    let refused = |why: String, hint: &str| RpcError::new("refused", why, hint);
    let dest = std::env::current_exe().and_then(|e| e.canonicalize()).map_err(|e| RpcError::internal(e.to_string()))?;
    let shown = dest.display().to_string();
    if let Some(manager) = client::installed_by() {
        let how = match manager {
            "homebrew" => "brew upgrade herdr-core",
            "nix" => "update it in your Nix configuration",
            "mise" => "mise upgrade herdr-core",
            _ => {
                return Err(refused(
                    format!("{} isn't writable by you", dest.parent().unwrap().display()),
                    "update herdr-core the way it was installed (package manager, or sudo)",
                ));
            }
        };
        return Err(refused(format!("{shown} belongs to {manager}"), how));
    }
    let was = format!("hc {}", env!("CARGO_PKG_VERSION"));
    let now = version_of(from).ok_or_else(|| refused(format!("{} is not a working hc", from.display()), ""))?;
    if older(&now, &was) && !force {
        return Err(refused(format!("{now} is older than this {was}"), "--force to go back anyway"));
    }
    // Staged beside it, checked, then swapped in: never a half-written hc.
    crate::platform::swap_in(from, &dest, |staged| version_of(staged).as_deref() == Some(now.as_str())).map_err(
        |e| match (e.kind(), crate::platform::bundle_update_hint()) {
            (std::io::ErrorKind::InvalidData, _) => refused(format!("the copy of {} does not run", from.display()), ""),
            (std::io::ErrorKind::InvalidInput, Some(hint)) => refused(e.to_string(), hint),
            _ => RpcError::internal(format!("cannot replace {shown}: {e}")),
        },
    )?;
    let restarted = match client::connect_existing(paths) {
        Some(stream) => {
            client::call(&stream, "server.restart", json!({}))?;
            wait_closed(&stream);
            true
        }
        None => false,
    };
    let status = call(paths, "server.status", json!({}))?;
    let human = format!(
        "{shown}: {was} -> {now}; core running again (pid {}), terminals kept{}",
        status["pid"],
        crate::platform::privacy_upgrade_notice(&dest)
    );
    Ok(Out {
        json: json!({"path": shown, "was": was, "now": now, "restarted": restarted, "pid": status["pid"]}),
        human,
    })
}
