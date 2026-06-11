use std::{env, fs};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ui");
    let current = Path::new(".").canonicalize().unwrap();
    Command::new("npx")
        .current_dir(PathBuf::from("ui"))
        .args(["tstl", "--luaBundleEntry", "main.ts", "--luaBundle", &format!("{}/out.lua", current.to_str().unwrap())])
        .status()
        .expect("UI compilation failed!");
}