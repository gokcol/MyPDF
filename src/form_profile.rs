//! My details: name, email, phone, address and so on, saved once on this
//! computer and used to fill forms. Fields are recognised by their names and
//! tooltips, in English and Turkish, and only empty fields are filled.
use crate::forms::{Field, FieldKind};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    FullName,
    FirstName,
    LastName,
    Email,
    Phone,
    Company,
    JobTitle,
    Street,
    City,
    PostalCode,
    Country,
    Website,
    /// Today's date, for fields that ask for the date of filling in.
    Today,
}
impl Detail {
    /// The details people keep; Today is not kept.
    pub const SAVED: [Detail; 12] = [
        Detail::FullName,
        Detail::FirstName,
        Detail::LastName,
        Detail::Email,
        Detail::Phone,
        Detail::Company,
        Detail::JobTitle,
        Detail::Street,
        Detail::City,
        Detail::PostalCode,
        Detail::Country,
        Detail::Website,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Detail::FullName => "Full name",
            Detail::FirstName => "First name",
            Detail::LastName => "Last name",
            Detail::Email => "Email",
            Detail::Phone => "Phone",
            Detail::Company => "Company",
            Detail::JobTitle => "Job title",
            Detail::Street => "Street address",
            Detail::City => "City",
            Detail::PostalCode => "Postal code",
            Detail::Country => "Country",
            Detail::Website => "Website",
            Detail::Today => "Today's date",
        }
    }
    /// What fields for this detail are called, without spaces or accents.
    /// Words of four letters or fewer must be a whole word of the name.
    fn names(self) -> &'static [&'static str] {
        match self {
            Detail::FullName => &[
                "name",
                "fullname",
                "yourname",
                "applicantname",
                "namesurname",
                "adsoyad",
                "adisoyadi",
                "adsoyadi",
                "adisoyad",
                "isimsoyisim",
                "isim",
                "adiniz",
            ],
            Detail::FirstName => &["firstname", "givenname", "forename", "ad", "adi"],
            Detail::LastName => &[
                "lastname",
                "surname",
                "familyname",
                "soyad",
                "soyadi",
                "soyisim",
            ],
            Detail::Email => &[
                "email",
                "emailaddress",
                "mail",
                "eposta",
                "epostaadresi",
                "elektronikposta",
            ],
            Detail::Phone => &[
                "phone",
                "phonenumber",
                "telephone",
                "tel",
                "mobile",
                "cell",
                "gsm",
                "telefon",
                "telefonnumarasi",
                "cep",
                "ceptelefonu",
            ],
            Detail::Company => &[
                "company",
                "companyname",
                "organization",
                "organisation",
                "employer",
                "business",
                "firm",
                "sirket",
                "sirketadi",
                "firma",
                "firmaadi",
                "kurum",
                "kurumadi",
                "kurulus",
            ],
            Detail::JobTitle => &[
                "jobtitle",
                "position",
                "role",
                "occupation",
                "unvan",
                "unvani",
                "gorev",
                "gorevi",
                "pozisyon",
                "meslek",
            ],
            Detail::Street => &[
                "address",
                "street",
                "streetaddress",
                "addressline1",
                "adres",
                "adresi",
                "sokak",
                "cadde",
                "mahalle",
            ],
            Detail::City => &["city", "town", "sehir", "il"],
            Detail::PostalCode => &["postalcode", "postcode", "zip", "zipcode", "postakodu"],
            Detail::Country => &["country", "ulke"],
            Detail::Website => &[
                "website",
                "web",
                "url",
                "homepage",
                "websitesi",
                "internetsitesi",
            ],
            Detail::Today => &[
                "date",
                "tarih",
                "today",
                "datesigned",
                "signeddate",
                "signaturedate",
                "imzatarihi",
                "bugun",
            ],
        }
    }
}

/// Words that mean a field is about someone or something else.
const OTHERS: &[&str] = &[
    "father",
    "mother",
    "spouse",
    "partner",
    "child",
    "children",
    "parent",
    "guardian",
    "emergency",
    "reference",
    "witness",
    "beneficiary",
    "recipient",
    "manager",
    "supervisor",
    "username",
    "file",
    "filename",
    "project",
    "product",
    "bank",
    "account",
    "baba",
    "anne",
    "es",
    "esi",
    "cocuk",
    "veli",
    "tanik",
    "referans",
    "acil",
    "yonetici",
    "proje",
    "urun",
    "banka",
    "birth",
    "dogum",
    "expiry",
    "expiration",
    "start",
    "end",
    "due",
    "issue",
    "gecerlilik",
    "baslangic",
    "bitis",
    "son",
    "password",
    "sifre",
];

/// The details, as they are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub full_name: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub phone: String,
    pub company: String,
    pub job_title: String,
    pub street: String,
    pub city: String,
    pub postal_code: String,
    pub country: String,
    pub website: String,
}
impl Profile {
    pub fn field(&mut self, detail: Detail) -> Option<&mut String> {
        Some(match detail {
            Detail::FullName => &mut self.full_name,
            Detail::FirstName => &mut self.first_name,
            Detail::LastName => &mut self.last_name,
            Detail::Email => &mut self.email,
            Detail::Phone => &mut self.phone,
            Detail::Company => &mut self.company,
            Detail::JobTitle => &mut self.job_title,
            Detail::Street => &mut self.street,
            Detail::City => &mut self.city,
            Detail::PostalCode => &mut self.postal_code,
            Detail::Country => &mut self.country,
            Detail::Website => &mut self.website,
            Detail::Today => return None,
        })
    }
    /// A detail's value. First and last name come from the full name when
    /// they are not given, and the other way round.
    pub fn value(&self, detail: Detail) -> String {
        let full = self.full_name.trim();
        let (first, last) = match full.rsplit_once(' ') {
            Some((first, last)) => (first.trim(), last.trim()),
            None => (full, ""),
        };
        let given = |text: &str, fallback: &str| {
            if text.trim().is_empty() {
                fallback.to_owned()
            } else {
                text.trim().to_owned()
            }
        };
        match detail {
            Detail::FullName => given(
                full,
                format!("{} {}", self.first_name.trim(), self.last_name.trim()).trim(),
            ),
            Detail::FirstName => given(&self.first_name, first),
            Detail::LastName => given(&self.last_name, last),
            Detail::Email => self.email.trim().to_owned(),
            Detail::Phone => self.phone.trim().to_owned(),
            Detail::Company => self.company.trim().to_owned(),
            Detail::JobTitle => self.job_title.trim().to_owned(),
            Detail::Street => self.street.trim().to_owned(),
            Detail::City => self.city.trim().to_owned(),
            Detail::PostalCode => self.postal_code.trim().to_owned(),
            Detail::Country => self.country.trim().to_owned(),
            Detail::Website => self.website.trim().to_owned(),
            Detail::Today => String::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        Detail::SAVED.iter().all(|d| self.value(*d).is_empty())
    }
}

/// Where the details are kept: `my_details.json` in the data folder.
pub fn file() -> PathBuf {
    crate::assets::data_directory().join("my_details.json")
}
pub fn load(path: &Path) -> Result<Profile> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Your saved details could not be read"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Profile::default()),
        Err(e) => Err(e).context("Your saved details could not be read"),
    }
}
pub fn save(path: &Path, profile: &Profile) -> Result<()> {
    let folder = path.parent().context("No folder for your details")?;
    std::fs::create_dir_all(folder)?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(profile)?)?;
    file.persist(path)?;
    Ok(())
}

/// Lowercase, Turkish letters as plain ones, and the words of a name: split
/// at spaces, punctuation, digits and lower-to-upper case changes.
fn words(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    let mut previous_lower = false;
    for c in text.chars() {
        let plain = match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'I' | 'İ' | 'i' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            c if c.is_ascii_alphabetic() => c.to_ascii_lowercase(),
            _ => ' ',
        };
        let boundary = plain == ' ' || (c.is_uppercase() && previous_lower);
        if boundary && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if plain != ' ' {
            word.push(plain);
        }
        previous_lower = c.is_lowercase();
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Which detail a field name or tooltip asks for, and how sure: the length
/// of the name that matched, so "company name" is a company, not a name.
fn detail_of(text: &str) -> Option<(Detail, usize)> {
    let words = words(text);
    if words.iter().any(|w| OTHERS.contains(&w.as_str())) {
        return None;
    }
    let joined: String = words.concat();
    // "Address line 2", "Phone 2" and "Signer 2 name" are not the first ones.
    if text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .any(|n| n.parse::<u32>() != Ok(1))
    {
        return None;
    }
    let mut best: Option<(Detail, usize)> = None;
    for detail in Detail::SAVED.into_iter().chain([Detail::Today]) {
        for name in detail.names() {
            let found = joined == *name
                || words.iter().any(|w| w == name)
                || (name.len() > 4 && joined.contains(name));
            // Dates only when that is all the field asks.
            let found = found && (detail != Detail::Today || joined == *name);
            if found && best.is_none_or(|(_, n)| name.len() > n) {
                best = Some((detail, name.len()));
            }
        }
    }
    best
}

/// Today's date here, as (year, month, day).
pub fn today() -> (i32, u32, u32) {
    #[cfg(not(target_os = "ios"))]
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    #[cfg(target_os = "ios")]
    let now = time::OffsetDateTime::now_utc();
    (now.year(), u8::from(now.month()) as u32, now.day() as u32)
}

/// A value proposed for an empty field.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub field: lopdf::ObjectId,
    pub name: String,
    pub detail: Detail,
    pub value: String,
}

/// Values for the empty fields that ask for one of the details. `today` is
/// the date to put in date fields, as (year, month, day).
pub fn suggest(bytes: &[u8], profile: &Profile, today: (i32, u32, u32)) -> Result<Vec<Suggestion>> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let fields = crate::forms::fields(bytes)?;
    let mut out = vec![];
    for field in fields {
        if !field.value.trim().is_empty() || field.reason.is_some() {
            continue;
        }
        let tooltip = crate::content::inherited(&doc, field.id, b"TU")
            .ok()
            .flatten()
            .and_then(|o| lopdf::decode_text_string(&o).ok())
            .unwrap_or_default();
        let local = field.name.rsplit('.').next().unwrap_or(&field.name);
        let Some((detail, _)) = [local, &tooltip]
            .into_iter()
            .filter_map(detail_of)
            .max_by_key(|(_, n)| *n)
        else {
            continue;
        };
        if let Some(value) = value_for(&field, profile, detail, today) {
            out.push(Suggestion {
                field: field.id,
                name: field.name.clone(),
                detail,
                value,
            });
        }
    }
    Ok(out)
}

/// The value as this field takes it, if it can.
fn value_for(
    field: &Field,
    profile: &Profile,
    detail: Detail,
    today: (i32, u32, u32),
) -> Option<String> {
    match (&field.kind, detail) {
        (FieldKind::Date { format }, Detail::Today) => {
            Some(crate::form_data::format_date(format, today))
        }
        (FieldKind::Text { .. }, Detail::Today) => {
            Some(crate::form_data::format_date("dd.mm.yyyy", today))
        }
        (_, Detail::Today) | (FieldKind::Date { .. }, _) => None,
        (FieldKind::Text { max, .. }, _) => {
            let value = profile.value(detail);
            (!value.is_empty() && max.is_none_or(|m| value.chars().count() <= m)).then_some(value)
        }
        (FieldKind::Choice { options, editable }, _) => {
            let value = profile.value(detail);
            if value.is_empty() {
                return None;
            }
            let lower = value.to_lowercase();
            options
                .iter()
                .find(|(v, l)| v.to_lowercase() == lower || l.to_lowercase() == lower)
                .map(|(v, _)| v.clone())
                .or_else(|| editable.then_some(value))
        }
        _ => None,
    }
}

/// Fill the chosen suggestions, as one change.
pub fn fill(bytes: &[u8], chosen: &[Suggestion]) -> Result<Vec<u8>> {
    let mut bytes = bytes.to_vec();
    for s in chosen {
        bytes = crate::forms::fill(&bytes, s.field, &s.value)
            .with_context(|| format!("{} could not be filled", s.name))?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_names_in_english_and_turkish_are_recognised() {
        let detail = |text: &str| detail_of(text).map(|(d, _)| d);
        for (text, expected) in [
            ("Name", Some(Detail::FullName)),
            ("Full name", Some(Detail::FullName)),
            ("FirstName", Some(Detail::FirstName)),
            ("txtLastName", Some(Detail::LastName)),
            ("Company name", Some(Detail::Company)),
            ("E-mail address", Some(Detail::Email)),
            ("Email Address", Some(Detail::Email)),
            ("Address line 1", Some(Detail::Street)),
            ("Address line 2", None),
            ("Signer 1 name", Some(Detail::FullName)),
            ("Signer 2 name", None),
            ("Phone 2", None),
            ("ZIP", Some(Detail::PostalCode)),
            ("Adı Soyadı", Some(Detail::FullName)),
            ("SOYADI", Some(Detail::LastName)),
            ("Adı", Some(Detail::FirstName)),
            ("E-posta", Some(Detail::Email)),
            ("Cep Telefonu", Some(Detail::Phone)),
            ("Şirket Adı", Some(Detail::Company)),
            ("Ülke", Some(Detail::Country)),
            ("Şehir", Some(Detail::City)),
            ("Unvanı", Some(Detail::JobTitle)),
            ("Date", Some(Detail::Today)),
            ("Tarih", Some(Detail::Today)),
            ("Date of birth", None),
            ("Doğum Tarihi", None),
            ("Start date", None),
            ("Father's name", None),
            ("Emergency contact phone", None),
            ("Anne Adı", None),
            ("File name", None),
            ("Text1", None),
            ("Signature", None),
        ] {
            assert_eq!(detail(text), expected, "{text}");
        }
    }

    #[test]
    fn first_and_last_names_come_from_the_full_name() {
        let mut profile = Profile {
            full_name: "Ayşe Nur Yılmaz".into(),
            ..Default::default()
        };
        assert_eq!(profile.value(Detail::FirstName), "Ayşe Nur");
        assert_eq!(profile.value(Detail::LastName), "Yılmaz");
        profile.first_name = "Ayşe".into();
        assert_eq!(profile.value(Detail::FirstName), "Ayşe");
        let parts = Profile {
            first_name: "Can".into(),
            last_name: "Demir".into(),
            ..Default::default()
        };
        assert_eq!(parts.value(Detail::FullName), "Can Demir");
        assert!(Profile::default().is_empty());
        assert!(!parts.is_empty());
    }

    #[test]
    fn details_are_kept_in_a_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("details").join("my_details.json");
        assert_eq!(load(&path).unwrap(), Profile::default());
        let profile = Profile {
            full_name: "Ayşe Yılmaz".into(),
            email: "ayse@example.com".into(),
            ..Default::default()
        };
        save(&path, &profile).unwrap();
        assert_eq!(load(&path).unwrap(), profile);
        std::fs::write(&path, b"not json").unwrap();
        assert!(load(&path).is_err());
    }
}
