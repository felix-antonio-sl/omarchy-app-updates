fn main() {
    println!("cargo:rerun-if-changed=native/alpm-wrapper.h");
    println!("cargo:rerun-if-changed=/usr/include/alpm.h");
    let lib = pkg_config::Config::new()
        .probe("libalpm")
        .expect("libalpm development headers required");
    println!("cargo:rustc-env=APP_UPDATES_ALPM_VERSION={}", lib.version);
    let bindings = bindgen::Builder::default()
        .header("native/alpm-wrapper.h")
        .allowlist_function("alpm_.*")
        .allowlist_type("alpm_.*")
        .allowlist_var("ALPM_.*")
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .expect("Generate bindings for the installed libalpm");
    bindings
        .write_to_file(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("alpm.rs"))
        .unwrap();
}
