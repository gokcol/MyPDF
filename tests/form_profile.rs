use mypdf_milestone0::form_design::{self, FieldType, Properties};
use mypdf_milestone0::form_profile::{self, Detail, Profile};
use mypdf_milestone0::{convert, forms};

fn field(bytes: &[u8], kind: FieldType, name: &str, tooltip: &str, y: f32) -> Vec<u8> {
    let mut properties = Properties::new(kind, name);
    properties.tooltip = tooltip.into();
    if kind == FieldType::Dropdown {
        properties.options = vec!["Germany".into(), "Türkiye".into(), "United Kingdom".into()];
    }
    if kind == FieldType::Date {
        properties.date_format = "dd.mm.yyyy".into();
    }
    form_design::add(bytes, 0, [60., y, 300., y + 22.], kind, &properties)
        .unwrap()
        .0
}
fn value(bytes: &[u8], name: &str) -> String {
    forms::fields(bytes)
        .unwrap()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap()
        .value
}

#[test]
fn my_details_fill_the_empty_fields_that_ask_for_them() {
    let mut pdf = convert::blank(595., 842.).unwrap();
    for (i, (kind, name, tooltip)) in [
        (FieldType::Text, "Text1", "Adı Soyadı"),
        (FieldType::Text, "Email", ""),
        (FieldType::Text, "Company", ""),
        (FieldType::Text, "FatherName", "Father's name"),
        (FieldType::Text, "Phone", ""),
        (FieldType::Dropdown, "Country", ""),
        (FieldType::Date, "Date", ""),
        (FieldType::Date, "BirthDate", "Date of birth"),
        (FieldType::Text, "Notes", ""),
        (FieldType::Text, "City", ""),
    ]
    .into_iter()
    .enumerate()
    {
        pdf = field(&pdf, kind, name, tooltip, 60. + i as f32 * 40.);
    }
    // A field that already holds something keeps it.
    let id = forms::fields(&pdf)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "Phone")
        .unwrap()
        .id;
    let pdf = forms::fill(&pdf, id, "0212 555 00 00").unwrap();
    let profile = Profile {
        full_name: "Ayşe Yılmaz".into(),
        email: "ayse@example.com".into(),
        phone: "0555 123 45 67".into(),
        country: "türkiye".into(),
        ..Default::default()
    };
    let suggestions = form_profile::suggest(&pdf, &profile, (2026, 10, 15)).unwrap();
    let found: Vec<(&str, Detail, &str)> = suggestions
        .iter()
        .map(|s| (s.name.as_str(), s.detail, s.value.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("Text1", Detail::FullName, "Ayşe Yılmaz"),
            ("Email", Detail::Email, "ayse@example.com"),
            ("Country", Detail::Country, "Türkiye"),
            ("Date", Detail::Today, "15.10.2026"),
        ],
        "Company and City are not known, the father's name and birth date are someone else's, and Phone is filled"
    );
    let filled = form_profile::fill(&pdf, &suggestions).unwrap();
    assert_eq!(value(&filled, "Text1"), "Ayşe Yılmaz");
    assert_eq!(value(&filled, "Country"), "Türkiye");
    assert_eq!(value(&filled, "Date"), "15.10.2026");
    assert_eq!(value(&filled, "Phone"), "0212 555 00 00");
    assert_eq!(value(&filled, "FatherName"), "");
    // Once filled, nothing more is proposed.
    assert!(
        form_profile::suggest(&filled, &profile, (2026, 10, 15))
            .unwrap()
            .is_empty()
    );
}
