extern crate bindgen;

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::fs;

fn main() {
    // Check if the target OS is macOS
    if cfg!(target_os = "macos") {
        // Use Homebrew to find the Opus library
        let brew_prefix = Command::new("brew")
            .arg("--prefix")
            .arg("opus")
            .output()
            .expect("Failed to execute brew")
            .stdout;
        let brew_prefix = String::from_utf8(brew_prefix).expect("Invalid UTF-8 output from brew");
        let brew_prefix = brew_prefix.trim();

        println!("Brew prefix: {}", brew_prefix);

        // Set the include path for the Opus headers
        let include_path = format!("{}/include/opus", brew_prefix);
        let lib_path = format!("{}/lib", brew_prefix);

        println!("cargo:include={}", include_path);
        println!("cargo:rustc-link-search=native={}", lib_path);

        // The bindgen::Builder is the main entry point to bindgen, and lets you build up options for the resulting bindings.
        let bindings = bindgen::Builder::default()
            // The input header we would like to generate bindings for.
            .header(format!("{}/include/opus/opus.h", brew_prefix))
            // Add the include path for the Opus headers
            .clang_arg(format!("-I{}", include_path))
            // Finish the builder and generate the bindings.
            .generate()
            // Unwrap the Result and panic on failure.
            .expect("Unable to generate bindings");

        // Write the bindings to the $OUT_DIR/bindings.rs file.
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        bindings
            .write_to_file(out_path.join("bindings.rs"))
            .expect("Couldn't write bindings!");
    }

    // Tell cargo to tell rustc to link the system opus shared library.
    println!("cargo:rustc-link-lib=opus");

    println!("OUT_DIR: {}", env::var("OUT_DIR").unwrap());

    // Write the OUT_DIR to a file
    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write("out_dir.txt", &out_dir).expect("Unable to write OUT_DIR to file");
} 