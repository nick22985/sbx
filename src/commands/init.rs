use std::path::Path;

use crate::config::Config;
use crate::docker;
use crate::flavor::{build_image, image_name, is_flavor, is_internal_flavor, list_flavors};
use crate::project::{private_write_dir, sbx_write_dir};
use crate::util::{die, log};

pub fn run(cwd: &Path, flavor: &str, private: bool) {
    if is_internal_flavor(flavor) {
        die(format!(
            "'{flavor}' isn't a project flavor - use `sbx {flavor}` to launch it directly"
        ));
    }
    if !is_flavor(flavor) {
        die(format!(
            "unknown flavor: {flavor} (have: {})",
            list_flavors().join(",")
        ));
    }
    let write_dir = if private {
        private_write_dir(cwd)
    } else {
        sbx_write_dir(cwd)
    };
    let mut cfg = Config::load_or_default(cwd);
    cfg.flavor = Some(flavor.to_string());
    let path = cfg
        .save_to_dir(&write_dir)
        .unwrap_or_else(|e| die(format!("write {}: {e}", write_dir.display())));
    log(format!(
        "marked {} as flavor={flavor}",
        path.display()
    ));
    if !docker::image_exists(&image_name(flavor)) {
        build_image(flavor, false);
    }
    log("ready. run 'sbx' to enter.");
    log(format!(
        "extra deps: create {}/Dockerfile starting with 'FROM {}'",
        write_dir.display(),
        image_name(flavor)
    ));
}
