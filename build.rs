use std::io::Result;

fn main() -> Result<()> {
    println!("cargo::rerun-if-changed=src/store.proto");

    prost_build::compile_protos(&["src/store.proto"], &["src/"])
}