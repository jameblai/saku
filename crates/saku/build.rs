use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "cargo:rustc-env=SAKU_VERSION={} ({sha})",
        env!("CARGO_PKG_VERSION")
    );
}
