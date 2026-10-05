fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/platform/macos/Native.swift");
        println!("cargo:rerun-if-changed=resources/macos/Info.plist");
        let out = std::env::var("OUT_DIR").expect("OUT_DIR");
        let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64",
            Ok("x86_64") => "x86_64",
            _ => panic!("unsupported macOS architecture"),
        };
        let status = std::process::Command::new("xcrun")
            .args([
                "swiftc",
                "-swift-version",
                "5",
                "-O",
                "-emit-library",
                "-module-name",
                "DictationMac",
                "-module-cache-path",
                &format!("{out}/swift-module-cache"),
                "-target",
                &format!("{arch}-apple-macosx13.0"),
                "-Xlinker",
                "-install_name",
                "-Xlinker",
                "@rpath/libDictationMac.dylib",
                "src/platform/macos/Native.swift",
                "-o",
                &format!("{out}/libDictationMac.dylib"),
            ])
            .status()
            .expect("install Xcode Command Line Tools (xcode-select --install)");
        assert!(
            status.success(),
            "Swift platform adapter compilation failed"
        );
        println!("cargo:rustc-link-search=native={out}");
        println!("cargo:rustc-link-lib=dylib=DictationMac");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{out}");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        // cargo run also needs the privacy declaration, even outside an app bundle.
        let plist = std::path::Path::new("resources/macos/Info.plist")
            .canonicalize()
            .unwrap();
        println!("cargo:rustc-link-arg-bin=dictation-hotkey-native=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }

    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=resources/tray-idle.ico");
    println!("cargo:rerun-if-changed=resources/tray-recording.ico");
    println!("cargo:rerun-if-changed=resources/tray-processing.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("compile Windows manifest");
    }
}
