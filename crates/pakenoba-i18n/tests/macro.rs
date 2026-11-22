//! t! マクロの展開を確かめる（英語のまま）。
use pakenoba_i18n::t;

#[test]
fn 英語のままなら_format_と同じ文になる() {
    pakenoba_i18n::set("en");
    let name = "box.pknb";
    let (done, total) = (2, 5);
    let w = 12.345f32;
    let plain: &'static str = t!("Save");
    assert_eq!(plain, "Save");
    assert_eq!(t!("Saved {name}."), "Saved box.pknb.");
    assert_eq!(t!("{} of {}", done, total), "2 of 5");
    assert_eq!(t!("Width {w:.1} mm"), "Width 12.3 mm");
    assert_eq!(t!("Height {h:.0} mm", h = w * 2.0), "Height 25 mm");
    assert_eq!(t!("{name} is {} of {total}", done,), "box.pknb is 2 of 5");
    assert_eq!(t!("{{literal}} {name}"), "{literal} box.pknb");
}
