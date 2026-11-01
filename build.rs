#[cfg(windows)]
fn main() {
    println!("cargo:rerun-if-changed=assets/wgopt.rc");
    println!("cargo:rerun-if-changed=assets/wgopt.ico");

    embed_resource::compile("assets/wgopt.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}

#[cfg(not(windows))]
fn main() {}
