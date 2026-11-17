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
    let (ok, stdout, stderr) = run_source_update(&script, &dir).map_err(|e| {
        RpcError::new("refused", format!("cannot run {}: {e}", script.display()), "check HC_SOURCE_DIR")
    })?;
    if !ok {
        // The script's own verdict when it gave one, else the end of the build output.
        let lines: Vec<&str> = stderr.iter().map(String::as_str).filter(|l| !l.trim().is_empty()).collect();
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
    Ok(source_outcome(&stdout, &dir))
}

/// Runs the script, passing its `progress:` lines straight on to this hc's
/// stderr (they exist only when the caller set HC_PROGRESS). Returns whether
/// it succeeded, its stdout and its stderr lines.
fn run_source_update(script: &std::path::Path, dir: &str) -> std::io::Result<(bool, String, Vec<String>)> {
    use std::io::{BufRead, Read, Write};
    let mut child = std::process::Command::new("sh")
        .arg(script)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take().expect("piped stderr");
    let lines = std::thread::spawn(move || {
        let mut kept = Vec::new();
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.starts_with("progress: ") {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "{line}");
                let _ = err.flush();
            }
            kept.push(line);
        }
        kept
    });
    let mut stdout = String::new();
    child.stdout.take().expect("piped stdout").read_to_string(&mut stdout)?;
    let status = child.wait()?;
    Ok((status.success(), stdout, lines.join().unwrap_or_default()))
}

/// What the script reported: `key=value` lines, then the installed version.
fn source_outcome(stdout: &str, dir: &str) -> Out {
    let lines: Vec<&str> = stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let now = lines.last().copied().unwrap_or_default().to_owned();
    let field = |key: &str| lines.iter().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).unwrap_or_default();
    let merged: u64 = field("merged").parse().unwrap_or(0);
    let rebuilt = field("rebuilt") == "1";
    let short = |sha: &str| sha.get(..7).unwrap_or(sha).to_owned();
    let human = match (rebuilt, merged) {
        (false, _) => format!("{now} is up to date: no new upstream commits ({})", short(field("to"))),
        (true, 0) => format!("rebuilt {} from {dir}: now {now}", short(field("to"))),
        (true, n) => {
            format!("merged {n} upstream commit(s), {} → {}: now {now}", short(field("from")), short(field("to")))
        }
    };
    let json = json!({"now": now, "updated": rebuilt, "merged": merged, "from": field("from"), "to": field("to"), "source": dir});
    Out { json, human }
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
