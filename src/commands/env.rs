use crate::env_file;
use crate::util::{die, env_file_path, log};

pub enum Action<'a> {
    List,
    Set { key: &'a str, value: &'a str },
    Unset(&'a str),
}

pub fn run(action: Action<'_>) {
    let path = env_file_path();
    match action {
        Action::List => {
            if !path.is_file() {
                log(format!("no env file at {}", path.display()));
                return;
            }
            for entry in env_file::parse_env_file(&path) {
                println!(
                    "{}={}",
                    entry.key,
                    env_file::mask_value(&entry.key, &entry.value)
                );
            }
        }
        Action::Set { key, value } => {
            if let Err(e) = env_file::set_var(key, value) {
                die(format!("set {key}: {e}"));
            }
            log(format!("set {key} in {}", path.display()));
        }
        Action::Unset(key) => {
            if let Err(e) = env_file::unset_var(key) {
                die(format!("unset {key}: {e}"));
            }
            log(format!("unset {key} in {}", path.display()));
        }
    }
}
