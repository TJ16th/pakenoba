//! ウィンドウのアイコンが読めること（読めないと、黙って既定のアイコンになるため）。

#[test]
fn ウィンドウのアイコンが読める() {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/pakenoba-icon-256.png")).expect("icon decodes");
    assert_eq!((icon.width, icon.height), (256, 256));
    assert!(icon.rgba.chunks(4).any(|p| p[3] == 0), "背景が透けている");
}
