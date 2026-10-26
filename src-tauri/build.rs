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
    tauri_build::build();
}
