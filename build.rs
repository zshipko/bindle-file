use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Instead of using .generate(), use the Builder for more control
    let config = cbindgen::Config::from_file("cbindgen.toml").unwrap_or_default();

    if let Ok(b) = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        b.write_to_file(PathBuf::from(crate_dir).join("include/bindle.h"));
    } else {
        eprintln!("WARNING: bindle.h not updated");
    }
}
