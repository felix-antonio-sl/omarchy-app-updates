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
    let flatpak = pkg_config::Config::new()
        .probe("flatpak")
        .expect("libflatpak development headers required");
    let builder=bindgen::Builder::default().header("native/flatpak-wrapper.h")
        .allowlist_function("flatpak_.*|g_signal_connect_data|g_object_unref|g_error_free|g_free|g_list_free|g_key_file_to_data|g_cancellable_new|g_cancellable_cancel")
        .allowlist_type("Flatpak.*|G.*").allowlist_var("FLATPAK_.*").prepend_enum_name(false).layout_tests(false).generate_comments(false);
    let args = flatpak
        .include_paths
        .iter()
        .map(|p| format!("-I{}", p.display()));
    builder
        .clang_args(args)
        .generate()
        .expect("Generate libflatpak bindings")
        .write_to_file(
            std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("flatpak.rs"),
        )
        .unwrap();
    println!("cargo:rerun-if-changed=native/flatpak-wrapper.h");
}
