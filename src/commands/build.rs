use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::docker;
use crate::flavor::{
    BASE_FLAVOR, build_image, build_image_streamed, image_name, image_up_to_date, list_all_flavors,
    nix_gid, nix_uid, project_image_tag,
};
use crate::project::{project_flavor, sbx_file};
use crate::util::{die, log};

pub fn run(cwd: &Path, no_cache: bool, flavor_arg: Option<&str>) {
    if let Some(f) = flavor_arg {
        if f == "all" {
            build_all(no_cache);
            return;
        }
        if !no_cache && image_up_to_date(f) {
            log(format!(
                "{} is up to date; skipping (use --no-cache to force)",
                image_name(f)
            ));
            return;
        }
        build_image(f, no_cache);
        return;
    }
    let (flavor, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("specify a flavor or run from a project with .sbx/flavor"));
    let project_df = sbx_file(&root, "Dockerfile");
    if !project_df.is_file() {
        build_image(&flavor, no_cache);
        return;
    }
    if !docker::image_exists(&image_name(&flavor)) {
        build_image(&flavor, false);
    }
    let img = project_image_tag(&flavor, &root);
    log(format!(
        "building project image {img} from {}",
        project_df.display()
    ));
    let uid = nix_uid();
    let gid = nix_gid();
    let mut cmd = Command::new("docker");
    cmd.args(["buildx", "build", "--load"]);
    if no_cache {
        cmd.arg("--no-cache");
    }
    cmd.args([
        "--build-arg",
        &format!("USER_UID={uid}"),
        "--build-arg",
        &format!("USER_GID={gid}"),
        "-t",
        &img,
        "-f",
    ])
    .arg(&project_df)
    .arg(project_df.parent().unwrap_or(&root));
    let status = cmd.status().unwrap_or_else(|e| die(format!("docker: {e}")));
    if !status.success() {
        die("docker build failed");
    }
}

fn build_all(no_cache: bool) {
    let flavors = list_all_flavors();
    if flavors.is_empty() {
        die("no flavors found");
    }
    let (to_build, skipped): (Vec<_>, Vec<_>) = flavors
        .into_iter()
        .partition(|f| no_cache || !image_up_to_date(f));
    for f in &skipped {
        log(format!("{} is up to date; skipping", image_name(f)));
    }
    if to_build.is_empty() {
        log("all flavors up to date (use --no-cache to force)");
        return;
    }
    let (base, leaves): (Vec<_>, Vec<_>) = to_build.into_iter().partition(|f| f == BASE_FLAVOR);
    if !base.is_empty() {
        log(format!("building {} first", image_name(BASE_FLAVOR)));
        build_image(BASE_FLAVOR, no_cache);
    }
    if leaves.is_empty() {
        return;
    }
    log(format!("building in parallel: {}", leaves.join(", ")));
    let out_lock = Arc::new(Mutex::new(()));
    let handles: Vec<_> = leaves
        .into_iter()
        .map(|f| {
            let lock = out_lock.clone();
            thread::spawn(move || {
                let res = build_image_streamed(&f, no_cache, &f, lock);
                (f, res)
            })
        })
        .collect();
    let mut failed = Vec::new();
    for h in handles {
        match h.join() {
            Ok((_, Ok(()))) => {}
            Ok((flavor, Err(e))) => failed.push(format!("{flavor}: {e}")),
            Err(_) => failed.push("build thread panicked".to_string()),
        }
    }
    if !failed.is_empty() {
        die(format!("some flavors failed:\n  {}", failed.join("\n  ")));
    }
}
