// The WebRTC audio processing build is Jay-C's (plugins/tauri-plugin-system-audio/build.rs), kept
// the same so a fix there can be copied here as is.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        webrtc_apm::build();
    }
}

/// Builds WebRTC's audio processing library (third_party/webrtc-audio-processing, BSD) with Meson
/// + MSVC, plus the small C shim in native/, and links them. Needs Python with `meson` and `ninja`
/// on PATH (`py -m pip install meson ninja`). The first build also downloads abseil-cpp.
mod webrtc_apm {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const VERSION: &str = "2.1";
    const ABSEIL_DIR: &str = "abseil-cpp-20240722.0";

    pub fn build() {
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let source = manifest.join("third_party").join("webrtc-audio-processing");
        println!("cargo:rerun-if-changed=native/jayc_apm.cpp");
        println!("cargo:rerun-if-changed=third_party/webrtc-audio-processing/meson.build");

        // MSVC fails with paths over 260 characters (abseil's object names are long), so build in
        // a short directory next to Cargo's target dir instead of OUT_DIR. It's shared between
        // debug/release builds and only rebuilt when missing.
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let target_dir = out_dir.ancestors().nth(4).expect("OUT_DIR is <target>/<profile>/build/<pkg>/out").to_path_buf();
        let root = target_dir.join(format!("webrtc-apm-{VERSION}"));
        let (src, build, install) = (root.join("src"), root.join("b"), root.join("i"));
        let library = install.join("lib").join("libwebrtc-audio-processing-2.a");

        if !library.exists() {
            if src.exists() {
                fs::remove_dir_all(&src).unwrap();
            }
            copy_dir(&source, &src);
            meson(&src, &[
                "setup",
                build.to_str().unwrap(),
                "--vsenv",
                "--buildtype=release",
                "--default-library=static",
                "-Db_vscrt=md",
                // MSVC needs C++20 for designated initializers (GCC/Clang accept them in C++17);
                // abseil must stay on C++17.
                "-Dcpp_std=c++20",
                "-Dabseil-cpp:cpp_std=c++17",
                &format!("--prefix={}", install.display()),
            ]);
            meson(&src, &["compile", "-C", build.to_str().unwrap()]);
            meson(&src, &["install", "-C", build.to_str().unwrap(), "--no-rebuild"]);
        }

        // Meson names MSVC static libraries `libX.a`; give rustc `X.lib` copies to link.
        let link_dir = root.join("link");
        fs::create_dir_all(&link_dir).unwrap();
        let mut libraries = vec![library];
        let abseil = build.join("subprojects").join(ABSEIL_DIR);
        for entry in fs::read_dir(&abseil).expect("abseil build directory") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "a") {
                libraries.push(path);
            }
        }
        for library in &libraries {
            let stem = library.file_stem().unwrap().to_str().unwrap();
            let name = stem.strip_prefix("lib").unwrap_or(stem);
            fs::copy(library, link_dir.join(format!("{name}.lib"))).unwrap();
            println!("cargo:rustc-link-lib=static={name}");
        }
        println!("cargo:rustc-link-search=native={}", link_dir.display());
        println!("cargo:rustc-link-lib=winmm");

        cc::Build::new()
            .cpp(true)
            .file(manifest.join("native").join("jayc_apm.cpp"))
            .include(install.join("include").join("webrtc-audio-processing-2"))
            .include(install.join("include"))
            .define("WEBRTC_WIN", None)
            .define("WEBRTC_LIBRARY_IMPL", None)
            .define("NOMINMAX", None)
            .define("_USE_MATH_DEFINES", None)
            .define("__STDC_FORMAT_MACROS", "1")
            .flag("/std:c++20")
            .flag("/EHsc")
            .compile("jayc_apm");
    }

    fn meson(cwd: &Path, args: &[&str]) {
        // No pkg-config: everything comes from bundled sources. A system one (e.g. MSYS2's) could
        // hand Meson MinGW-built libraries that don't link with MSVC, and a broken one aborts setup.
        let status = Command::new("meson").args(args).env("PKG_CONFIG", "jayc-no-pkg-config").current_dir(cwd).status().unwrap_or_else(|e| {
            panic!(
                "couldn't run meson ({e}). Building Jay-A on Windows needs Python with Meson and Ninja: \
                 `py -m pip install --user meson ninja`, then add Python's user Scripts folder to PATH."
            )
        });
        assert!(status.success(), "meson {args:?} failed");
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }
}
