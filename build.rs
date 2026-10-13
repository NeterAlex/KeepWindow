fn main() {
    println!("cargo:rerun-if-changed=assets/keep-window.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/keep-window.ico")
            .set("ProductName", "KeepWindow")
            .set("FileDescription", "KeepWindow - Window Protection")
            .compile()
            .expect("failed to embed Windows application resources");
    }
}
