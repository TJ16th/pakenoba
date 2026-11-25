//! Windows の実行ファイルに、アイコンと版の情報（エクスプローラーのプロパティに出るもの）を埋め込む。

fn main() {
    println!("cargo:rerun-if-changed=assets/pakenoba.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/pakenoba.ico")
        .set("ProductName", "PAKENOBA")
        // タスクマネージャーなどに出る名前。
        .set("FileDescription", "PAKENOBA")
        .set("OriginalFilename", "pakenoba.exe")
        .set("LegalCopyright", "© TJ16th");
    if let Err(e) = res.compile() {
        // アイコンが入らないだけで、アプリは動く。ビルドは止めずに知らせる。
        println!("cargo:warning=アイコンを埋め込めませんでした: {e}");
    }
}
