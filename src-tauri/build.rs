fn main() {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=normal"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=TASKPULSE_BUILD_SHA={sha}");
    println!("cargo:rustc-env=TASKPULSE_BUILD_DIRTY={dirty}");
    println!(
        "cargo:rustc-env=TASKPULSE_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_default()
    );
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../src");
    // Cargo-only native verification embeds the already verified Bun output.
    // Invalidate the embed when only generated assets or their build config change.
    println!("cargo:rerun-if-changed=../svelte.config.js");
    println!("cargo:rerun-if-changed=../package.json");
    if std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some() {
        println!("cargo:rerun-if-changed=../build");
    }
    build_windows_legacy_bridge();
    // The checked-in application permissions make unlisted commands fail closed.
    tauri_build::build();
}

fn build_windows_legacy_bridge() {
    use std::{env, path::PathBuf, process::Command};
    println!("cargo:rerun-if-changed=windows/legacy_bridge.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_none()
    {
        return;
    }
    let target = env::var("TARGET").expect("Cargo target");
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => panic!("Unsupported Windows architecture for the compatibility bridge"),
    };
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out = root
        .join("target")
        .join(format!("taskpulse-legacy-{arch}.exe"));
    std::fs::create_dir_all(out.parent().unwrap()).expect("Create bridge output directory");
    let mut rustc = Command::new(env::var_os("RUSTC").expect("Cargo compiler"));
    rustc
        .args([
            "--edition",
            "2021",
            "--target",
            &target,
            "-C",
            "opt-level=s",
            "-C",
            "strip=symbols",
            "-C",
            "panic=abort",
        ])
        .arg(root.join("windows/legacy_bridge.rs"))
        .arg("-o")
        .arg(&out);
    if let Some(linker) = env::var_os("RUSTC_LINKER") {
        rustc
            .arg("-C")
            .arg(format!("linker={}", linker.to_string_lossy()));
    }
    if target.ends_with("-gnu") {
        rustc.args(["-C", "link-self-contained=yes"]);
    }
    assert!(
        rustc
            .status()
            .expect("Build Windows compatibility bridge")
            .success(),
        "Windows compatibility bridge did not build"
    );
}
