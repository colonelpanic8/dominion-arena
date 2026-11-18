use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .env("TZ", "UTC")
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn watch(path: &str) {
    if Path::new(path).exists() {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn main() {
    for var in ["DOMINION_COMMIT", "DOMINION_COMMIT_TIME"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let given = |var| std::env::var(var).ok().filter(|v| !v.is_empty());
    let (commit, time) = match (given("DOMINION_COMMIT"), given("DOMINION_COMMIT_TIME")) {
        (Some(c), Some(t)) => (c, t),
        _ => {
            if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
                watch(&format!("{dir}/HEAD"));
            }
            if let (Some(common), Some(head)) = (
                git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
                git(&["symbolic-ref", "-q", "HEAD"]),
            ) {
                watch(&format!("{common}/{head}"));
                watch(&format!("{common}/packed-refs"));
            }
            (
                git(&["rev-parse", "HEAD"]).unwrap_or_default(),
                git(&[
                    "log",
                    "-1",
                    "--format=%cd",
                    "--date=format-local:%Y%m%d%H%M%S",
                ])
                .unwrap_or_default(),
            )
        }
    };
    println!("cargo:rustc-env=DOMINION_COMMIT={commit}");
    println!("cargo:rustc-env=DOMINION_COMMIT_TIME={time}");
}
