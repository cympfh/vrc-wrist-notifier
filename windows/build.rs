// exe のアイコン（エクスプローラーやスタートアップの一覧に出る）。Windows 向けのときだけ
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        r.set("FileDescription", "vrc-wrist-notifier");
        r.set("ProductName", "vrc-wrist-notifier");
        r.compile().expect("リソースを埋め込めませんでした");
    }
}
