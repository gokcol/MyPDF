//! Sample documents for the README screenshots: two versions of an agreement,
//! a report and a supplier form. All names and figures are made up.
//!
//!   cargo run --release --example demo_docs -- <folder>
use mypdf_milestone0::{
    convert,
    form_layout::{self, CellKind, Column, Layout},
};

const AGREEMENT: &str = "# Service Agreement\n\nThis agreement is made between Northwind Ltd and the Client.\n\n## 1. Services\n\nNorthwind delivers the monthly report by the fifth working day of each month and keeps the data it receives confidential.\n\n## 2. Payment\n\nPayment is due within thirty days of the invoice date.\n\nLate payments carry a fee of 2% per month.\n\n## 3. Term\n\nThis agreement lasts for twelve months and renews automatically.\n\n## 4. Disputes\n\nDisputes are settled by the courts of Istanbul.";

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output folder"));
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        out.join("Service Agreement v1.pdf"),
        convert::text_to_pdf(AGREEMENT, true)?,
    )?;
    let v2 = AGREEMENT
        .replace("fifth", "third")
        .replace("thirty days", "sixty days")
        .replace("Late payments carry a fee of 2% per month.\n\n", "")
        .replace(
            "renews automatically.",
            "renews automatically.\n\nEither party may end it with sixty days' notice.",
        );
    std::fs::write(
        out.join("Service Agreement v2.pdf"),
        convert::text_to_pdf(&v2, true)?,
    )?;

    let report = "# Quarterly Report\n\nNorthwind Ltd · Third quarter\n\n## Summary\n\nRevenue grew by 12% compared with the second quarter, driven by new service contracts in Istanbul and Ankara. Operating costs stayed within the budget agreed in January.\n\nCustomer satisfaction rose to 4.6 out of 5, the highest score since the survey began. The support team answered 94% of requests within one working day.\n\n## Projects\n\nThe new reporting portal went live in August. Clients now receive their monthly reports two days earlier, and paper copies are no longer printed.\n\nThe warehouse move is on schedule and will finish in November. Deliveries continue from the old site until then.\n\n## Next quarter\n\nThe priorities are the annual audit, the renewal of the three largest service contracts, and training for the new support staff.\n\nThe board meets on 12 December to approve the budget for next year.";
    std::fs::write(
        out.join("Quarterly Report.pdf"),
        convert::text_to_pdf(report, true)?,
    )?;

    let form = convert::text_to_pdf(
        "# Supplier Registration\n\nPlease complete this form and return it to the purchasing team.",
        true,
    )?;
    let (form, _) = form_layout::insert(
        &form,
        0,
        [52., 130., 543., 300.],
        &Layout::Labeled {
            labels: ["Full name", "Company", "Email", "Phone", "Date"]
                .map(String::from)
                .to_vec(),
        },
    )?;
    let (form, _) = form_layout::insert(
        &form,
        0,
        [52., 330., 543., 420.],
        &Layout::Checklist {
            items: [
                "ISO 9001 certificate attached",
                "Bank details attached",
                "Confidentiality agreement signed",
            ]
            .map(String::from)
            .to_vec(),
        },
    )?;
    let (form, _) = form_layout::insert(
        &form,
        0,
        [52., 450., 543., 590.],
        &Layout::Table {
            columns: vec![
                Column::new("Product", CellKind::Text, 2.),
                Column::new("Quantity", CellKind::Number, 1.),
                Column::new("Delivery", CellKind::Date, 1.),
            ],
            rows: 3,
            header: true,
            lines: true,
        },
    )?;
    let (form, _) = form_layout::insert(
        &form,
        0,
        [52., 630., 543., 760.],
        &Layout::Signatures { signers: 2 },
    )?;
    std::fs::write(out.join("Supplier Registration.pdf"), form)?;
    Ok(())
}
