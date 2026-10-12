fn main() {
    use sha2::{Digest, Sha256};
    fn gather(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                gather(&entry.unwrap().path(), files);
            }
        } else if path.is_file() {
            files.push(path.into());
        }
    }
    let mut files = vec![];
    for path in [
        "src",
        "native",
        "catalog",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
    ] {
        gather(std::path::Path::new(path), &mut files);
    }
    files.sort();
    let mut hash = Sha256::new();
    for path in files {
        let name = path.to_str().unwrap().as_bytes();
        let bytes = std::fs::read(&path).unwrap();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    println!(
        "cargo:rustc-env=APP_UPDATES_SOURCE_DIGEST={:x}",
        hash.finalize()
    );
    println!("cargo:rerun-if-changed=native/alpm-wrapper.h");
    println!("cargo:rerun-if-changed=/usr/include/alpm.h");
    let lib = pkg_config::Config::new()
        .probe("libalpm")
        .expect("libalpm development headers required");
    println!("cargo:rustc-env=APP_UPDATES_ALPM_VERSION={}", lib.version);
    let bindings = bindgen::Builder::default()
        .header("native/alpm-wrapper.h")
        .allowlist_function("alpm_.*|free")
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
    pkg_config::Config::new()
        .probe("libarchive")
        .expect("libarchive headers required");
    bindgen::Builder::default().header("native/archive-wrapper.h")
        .allowlist_function("archive_read_.*|archive_entry_pathname|archive_entry_filetype|archive_entry_symlink|archive_entry_hardlink|archive_entry_size|archive_error_string")
        .allowlist_type("archive|archive_entry").allowlist_var("ARCHIVE_.*|AE_IF.*|APP_AE_.*")
        .layout_tests(false).generate_comments(false).generate().expect("Generate libarchive bindings")
        .write_to_file(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("archive.rs")).unwrap();
    println!("cargo:rerun-if-changed=/usr/include/archive.h");
    println!("cargo:rerun-if-changed=/usr/include/archive_entry.h");
}
