fn main() {
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
