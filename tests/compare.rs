use mypdf_milestone0::compare::{self, Kind};
use mypdf_milestone0::{convert, viewer};

fn pages(bytes: &[u8]) -> Vec<viewer::PageText> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    let cache = hayro::hayro_interpret::InterpreterCache::new();
    pdf.pages()
        .iter()
        .map(|p| viewer::extract_page(p, &cache))
        .collect()
}
fn version(text: &str) -> Vec<viewer::PageText> {
    pages(&convert::text_to_pdf(text, false).unwrap())
}

#[test]
fn words_carry_their_page_and_place() {
    let text = version("Payment is due in thirty days.\nLate fees apply.");
    let words = compare::words(&text);
    let names: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(
        names,
        [
            "Payment", "is", "due", "in", "thirty", "days.", "Late", "fees", "apply."
        ]
    );
    assert!(words.iter().all(|w| w.page == 0));
    // Left to right on the first line, the second line lower.
    assert!(words[0].rect[2] <= words[1].rect[0]);
    assert!(words[6].rect[1] > words[0].rect[3] - 1.);
    assert!(words[0].rect[0] > 40. && words[0].rect[0] < 60.);
}

#[test]
fn identical_versions_have_no_changes() {
    let text = "Section 1\nThe supplier delivers the goods.\nSection 2\nThe buyer pays.";
    assert!(
        compare::compare(&version(text), &version(text))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn changed_added_and_removed_words_are_found() {
    let old = version(
        "Agreement\nPayment is due in thirty days.\nThe seller keeps the goods insured.\nDisputes go to the courts of Ankara.",
    );
    let new = version(
        "Agreement\nPayment is due in sixty days.\nDisputes go to the courts of Ankara.\nThis agreement starts on signing.",
    );
    let changes = compare::compare(&old, &new).unwrap();
    assert_eq!(changes.len(), 3, "{changes:#?}");
    assert_eq!(changes[0].kind, Kind::Changed);
    assert_eq!(changes[0].old_text(), "thirty");
    assert_eq!(changes[0].new_text(), "sixty");
    assert_eq!(changes[1].kind, Kind::Removed);
    assert_eq!(changes[1].old_text(), "The seller keeps the goods insured.");
    assert!(changes[1].new.is_empty());
    assert_eq!(changes[2].kind, Kind::Added);
    assert_eq!(changes[2].new_text(), "This agreement starts on signing.");
    // The changed word is highlighted where it is on each page.
    let (o, n) = (&changes[0].old[0], &changes[0].new[0]);
    assert!((o.rect[1] - n.rect[1]).abs() < 1., "same line");
    assert!(o.rect[0] > 100.);
}

#[test]
fn changes_across_pages_name_their_page() {
    let mut old = String::new();
    for i in 0..120 {
        old.push_str(&format!("Clause {i} stays exactly as it was written.\n"));
    }
    let new = old.replace("Clause 100 stays", "Clause 100 changes");
    let (old, new) = (version(&old), version(&new));
    assert!(old.len() > 1);
    let changes = compare::compare(&old, &new).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].old_text(), "stays");
    assert_eq!(changes[0].new_text(), "changes");
    assert_eq!(changes[0].page(), changes[0].old[0].page);
    assert!(changes[0].page() > 0);
}

#[test]
fn scans_without_text_cannot_be_compared() {
    let blank = pages(&convert::blank(595., 842.).unwrap());
    let error = compare::compare(&blank, &blank).unwrap_err();
    assert!(error.contains("Recognize Text"), "{error}");
    // A version with text against an empty one: everything was added.
    let changes = compare::compare(&blank, &version("New text")).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, Kind::Added);
    assert_eq!(changes[0].old_at, None);
}
