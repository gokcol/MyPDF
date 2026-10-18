//! On Windows, MyPDF.exe carries its icon and version details (product name,
//! version, copyright), which Explorer shows and code signing checks.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(mypdf_apple_translation)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/windows/MyPDF.ico");
    println!("cargo:rerun-if-changed=native/translation/TranslationBridge.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_apple_translation();
    }
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/MyPDF.ico")
            .set("ProductName", "MyPDF")
            .set("FileDescription", "MyPDF")
            .set("CompanyName", "MyPDF contributors")
            .set(
                "LegalCopyright",
                "© 2026 MyPDF contributors · MIT OR Apache-2.0",
            );
        resource
            .compile()
            .expect("Windows icon and version details could not be added");
    }
}

fn build_apple_translation() {
    use std::{path::PathBuf, process::Command};
    // iOS and Windows builds never compile or link the macOS/AppKit bridge.
    let sdk_output = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output();
    let Ok(sdk_output) = sdk_output else {
        println!("cargo:warning=Apple translation disabled: Xcode is unavailable");
        return;
    };
    if !sdk_output.status.success() {
        println!("cargo:warning=Apple translation disabled: macOS SDK is unavailable");
        return;
    }
    let sdk = PathBuf::from(String::from_utf8_lossy(&sdk_output.stdout).trim());
    let module =
        sdk.join("System/Library/Frameworks/Translation.framework/Modules/Translation.swiftmodule");
    let supports_installed_models = std::fs::read_dir(module)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "swiftinterface")
                && std::fs::read_to_string(entry.path())
                    .is_ok_and(|contents| contents.contains("init(installedSource"))
        });
    if !supports_installed_models {
        println!("cargo:warning=Apple translation disabled: Xcode 26 or newer is required");
        return;
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => return,
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    // Share the Swift compiler cache within this Cargo target/profile instead
    // of retaining another copy for every build-script fingerprint. Swift keys
    // its cache entries by compiler/SDK/options; Cargo's output tree still keeps
    // different target architectures separate.
    let module_cache = out
        .ancestors()
        .nth(3)
        .expect("Cargo profile output directory")
        .join("mypdf-swift-module-cache");
    let object = out.join("TranslationBridge.o");
    let library = out.join("libmypdf_translation.a");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-object",
            "-whole-module-optimization",
            "-O",
            "-module-name",
            "MyPDFTranslation",
            "-target",
            &format!("{arch}-apple-macosx12.0"),
            "-Xfrontend",
            "-disable-autolink-frameworks",
            "-sdk",
        ])
        .arg(&sdk)
        .args(["-module-cache-path"])
        .arg(module_cache)
        .args(["native/translation/TranslationBridge.swift", "-o"])
        .arg(&object)
        .status()
        .expect("run Swift compiler");
    assert!(
        status.success(),
        "Apple translation bridge compilation failed"
    );
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(&library)
        .arg(&object)
        .status()
        .expect("archive Swift bridge");
    assert!(status.success(), "Apple translation bridge archive failed");
    println!("cargo:rustc-cfg=mypdf_apple_translation");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=mypdf_translation");
    println!(
        "cargo:rustc-link-search=native={}",
        sdk.join("usr/lib/swift").display()
    );
    let swiftc = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .expect("find Swift compiler");
    let swiftc = PathBuf::from(String::from_utf8_lossy(&swiftc.stdout).trim());
    let toolchain = swiftc
        .parent()
        .and_then(|path| path.parent())
        .expect("Swift toolchain path");
    println!(
        "cargo:rustc-link-search=native={}",
        toolchain.join("lib/swift/macosx").display()
    );
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=SwiftUI");
    // New frameworks must stay weak-linked so the rest of MyPDF still launches
    // on its supported macOS 12 baseline. Swift runtime availability gates calls.
    for framework in ["Translation", "_Translation_SwiftUI"] {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,{framework}");
    }
}
