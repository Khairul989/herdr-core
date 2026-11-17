fn main() {
    println!("cargo:rerun-if-changed=../../distribution/macos-Info.plist");
    for name in ["HC_MACOS_BUNDLE_ID", "HC_MACOS_TEAM_ID"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let (identifier, team) = match (std::env::var("HC_MACOS_BUNDLE_ID"), std::env::var("HC_MACOS_TEAM_ID")) {
        (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent)) => {
            ("dev.herdr.core".to_owned(), String::new())
        }
        (Ok(identifier), Ok(team)) => {
            assert!(
                identifier != "dev.herdr.core"
                    && identifier.contains('.')
                    && identifier.split('.').all(|part| {
                        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    }),
                "HC_MACOS_BUNDLE_ID must be a distinct local bundle identifier, not dev.herdr.core"
            );
            assert!(
                team.len() == 10 && team.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
                "HC_MACOS_TEAM_ID must be a 10-character Apple team identifier"
            );
            (identifier, team)
        }
        _ => panic!("set HC_MACOS_BUNDLE_ID and HC_MACOS_TEAM_ID together for a local source build"),
    };
    println!("cargo:rustc-env=HC_MACOS_BUNDLE_ID={identifier}");
    println!("cargo:rustc-env=HC_MACOS_TEAM_ID={team}");
    let plist = include_str!("../../distribution/macos-Info.plist")
        .replace("@VERSION@", &std::env::var("CARGO_PKG_VERSION").unwrap())
        .replace("<string>dev.herdr.core</string>", &format!("<string>{identifier}</string>"));
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("Info.plist");
    std::fs::write(&path, plist).unwrap();
    // Separate arguments preserve paths containing spaces.
    for arg in ["-sectcreate", "__TEXT", "__info_plist"] {
        println!("cargo:rustc-link-arg-bin=herdr-core=-Wl,{arg}");
    }
    println!("cargo:rustc-link-arg-bin=herdr-core={}", path.display());
}
