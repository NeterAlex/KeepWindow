fn main() {
    println!("cargo:rerun-if-changed=assets/keep-window.ico");
    println!("cargo:rerun-if-changed=locales/zh-CN.json");
    println!("cargo:rerun-if-changed=locales/en-US.json");
    let zh: serde_json::Value =
        serde_json::from_str(include_str!("locales/zh-CN.json")).expect("invalid zh-CN locale");
    let en: serde_json::Value =
        serde_json::from_str(include_str!("locales/en-US.json")).expect("invalid en-US locale");
    let zh_keys = zh.as_object().expect("zh-CN locale must be an object");
    let en_keys = en.as_object().expect("en-US locale must be an object");
    assert_eq!(
        zh_keys.keys().collect::<Vec<_>>(),
        en_keys.keys().collect::<Vec<_>>(),
        "locale keys differ"
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/keep-window.ico")
            .set("ProductName", "KeepWindow")
            .set("FileDescription", "KeepWindow - Window Protection")
            .compile()
            .expect("failed to embed Windows application resources");
    }
}
