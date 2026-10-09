//! The embedded fonts must be found under the family names the renderer maps to.
#[test]
fn embedded_families_resolve() {
    let mut db = resvg::usvg::fontdb::Database::new();
    for f in ["NotoSans", "SourceSerif4", "Handlee-Regular", "Knewave-Regular", "Griffy-Regular"] {
        let path = format!("{}/fonts/{}.ttf", env!("CARGO_MANIFEST_DIR"), f);
        db.load_font_data(std::fs::read(path).unwrap());
    }
    let families: Vec<String> = db.faces().flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).collect();
    for want in ["Noto Sans", "Source Serif 4", "Handlee", "Knewave", "Griffy"] {
        assert!(families.iter().any(|f| f == want), "{want} missing from {families:?}");
    }
}
