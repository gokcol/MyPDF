//! Repeatable image-history churn without touching user documents or settings.
//! Run: cargo run --release --example history_memory_stress -- --cycles 18 --hold-seconds 5
//! The printed PID and stage holds allow `vmmap -summary PID` / process sampling.
use anyhow::{Result, ensure};
use clap::Parser;
use mypdf_milestone0::{
    annotations::{Annotation, Kind, Session, Style},
    convert, image_background, image_effects,
};
use std::{io::Cursor, sync::Arc, time::Duration};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 18)]
    cycles: usize,
    #[arg(long, default_value_t = 5)]
    hold_seconds: u64,
    #[arg(long, default_value_t = 48)]
    budget_mib: usize,
}

fn stage(name: &str, session: Option<&Session>, seconds: u64) {
    println!(
        "{}",
        serde_json::json!({
            "stage": name,
            "pid": std::process::id(),
            "retained_bytes": session.map(Session::retained_bytes),
            "current_bytes": session.map(Session::current_retained_bytes),
            "hold_seconds": seconds,
        })
    );
    std::thread::sleep(Duration::from_secs(seconds));
}

fn picture(seed: u32) -> Result<Vec<u8>> {
    let mut state = seed;
    let picture = image::RgbaImage::from_fn(2048, 960, |x, y| {
        if x < 16 || y < 16 || x >= 2032 || y >= 944 {
            image::Rgba([255; 4])
        } else {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image::Rgba([state as u8, (state >> 8) as u8, (state >> 16) as u8, 255])
        }
    });
    let mut output = Cursor::new(Vec::new());
    picture.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!((4..=100).contains(&args.cycles), "Use 4–100 cycles");
    ensure!(
        (8..=384).contains(&args.budget_mib),
        "Use an 8–384 MiB history budget"
    );
    ensure!(
        args.hold_seconds <= 30,
        "Stage holds must be at most 30 seconds"
    );
    let budget = args.budget_mib * 1024 * 1024;
    stage("baseline", None, args.hold_seconds);
    let mut session = Session::new(convert::blank(595., 842.)?.into(), vec![], true);
    let mut buffers = Vec::new();
    for i in 0..args.cycles {
        let original = picture(i as u32 + 123)?;
        let changed = if i % 2 == 0 {
            image_background::remove(&original, image_background::Options::default())?
        } else {
            image_effects::raster(
                &Arc::new(original),
                image_effects::RasterOptions {
                    blur: 2.,
                    ..Default::default()
                },
            )?
        };
        let png = Arc::new(changed);
        buffers.push(Arc::downgrade(&png));
        let mark = Annotation::new(
            0,
            [40., 40., 552., 296.],
            Kind::Image { png },
            Style::default(),
        );
        session.commit(vec![mark])?;
        session.trim_history_to_budget(budget);
        ensure!(
            session.retained_bytes() <= budget.max(session.current_retained_bytes()),
            "History exceeded its budget"
        );
        // Undo and Redo must move the cursor, not duplicate the image buffers.
        let bytes = session.retained_bytes();
        if session.can_undo() {
            let expected = session.annotations.clone();
            session.undo();
            ensure!(
                session.retained_bytes() == bytes,
                "Undo duplicated image storage"
            );
            session.redo();
            ensure!(
                session.annotations == expected && session.retained_bytes() == bytes,
                "Redo did not restore the exact image"
            );
        }
        println!(
            "{}",
            serde_json::json!({"cycle": i + 1, "retained_bytes": session.retained_bytes(), "history_states": session.history().1.len()})
        );
    }
    ensure!(
        buffers.iter().any(|b| b.upgrade().is_none()),
        "Increase cycles or reduce budget to exercise eviction"
    );
    stage("after_image_churn", Some(&session), args.hold_seconds);
    drop(session);
    ensure!(
        buffers.iter().all(|b| b.upgrade().is_none()),
        "Closed document retained source images"
    );
    // The process-wide blur appearance cache is independently bounded; it may
    // remain warm after this stage, just as it does between document sessions.
    stage("after_document_close", None, args.hold_seconds);
    println!("Image-history budget, Undo/Redo, and source-buffer release checks passed.");
    Ok(())
}
