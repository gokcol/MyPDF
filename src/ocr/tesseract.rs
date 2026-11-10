//! Bundled Tesseract subprocess: no PATH lookup, cloud calls or temporary PDFs.
use super::Line;
use anyhow::{Context as _, Result, ensure};

/// TSV coordinates refer to rendered pixels; PDF geometry uses displayed points.
fn parse_tsv(tsv: &str, pixels: [u32; 2], points: [f32; 2]) -> Result<Vec<Line>> {
    ensure!(
        pixels[0] > 0 && pixels[1] > 0,
        "Invalid OCR image dimensions"
    );
    ensure!(
        tsv.lines()
            .next()
            .is_some_and(|l| l.starts_with("level\tpage_num\t")),
        "Invalid OCR output"
    );
    let mut lines: Vec<Line> = Vec::new();
    let mut previous = None;
    for row in tsv.lines().skip(1) {
        let columns: Vec<_> = row.splitn(12, '\t').collect();
        if columns.len() != 12 || columns[0] != "5" || columns[11].trim().is_empty() {
            continue;
        }
        let number = |i: usize| -> Result<u32> {
            columns[i].parse().context("Invalid OCR word coordinates")
        };
        let key = (number(1)?, number(2)?, number(3)?, number(4)?);
        let (x, y, w, h) = (number(6)?, number(7)?, number(8)?, number(9)?);
        ensure!(
            w > 0 && h > 0 && x.saturating_add(w) <= pixels[0] && y.saturating_add(h) <= pixels[1],
            "OCR word lies outside the page"
        );
        let rect = [
            x as f32 / pixels[0] as f32 * points[0],
            y as f32 / pixels[1] as f32 * points[1],
            (x + w) as f32 / pixels[0] as f32 * points[0],
            (y + h) as f32 / pixels[1] as f32 * points[1],
        ];
        if previous == Some(key) {
            let line = lines.last_mut().expect("previous line exists");
            line.text.push(' ');
            line.text.push_str(columns[11]);
            line.rect = [
                line.rect[0].min(rect[0]),
                line.rect[1].min(rect[1]),
                line.rect[2].max(rect[2]),
                line.rect[3].max(rect[3]),
            ];
        } else {
            lines.push(Line {
                text: columns[11].into(),
                rect,
            });
            previous = Some(key);
        }
    }
    Ok(lines)
}

#[cfg(target_os = "windows")]
pub(super) fn read(
    png: &[u8],
    pixels: [u32; 2],
    points: [f32; 2],
    cancellation: &super::Cancellation,
) -> Result<Vec<Line>> {
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    cancellation.check()?;
    let runtime = std::env::var_os("MYPDF_OCR_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(
            std::env::current_exe()?
                .parent()
                .context("Application directory missing")?
                .join("ocr"),
        );
    let runtime = runtime.canonicalize().context(
        "The bundled OCR engine is missing. Reinstall the complete MyPDF Windows package.",
    )?;
    let languages = std::fs::read_to_string(runtime.join("languages.txt"))
        .context("The OCR language list is missing. Reinstall MyPDF.")?;
    let languages = languages.trim();
    ensure!(
        !languages.is_empty()
            && languages
                .split('+')
                .all(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_lowercase())),
        "Invalid OCR language list"
    );
    for language in languages.split('+') {
        ensure!(
            runtime
                .join("tessdata")
                .join(format!("{language}.traineddata"))
                .is_file(),
            "The {language} OCR model is missing. Reinstall MyPDF."
        );
    }
    // Relative tessdata avoids narrow-character path handling inside Tesseract.
    // Windows' wide-character process API handles the executable and working dir.
    let mut child = Command::new(runtime.join("tesseract.exe"))
        .current_dir(&runtime)
        .args([
            "stdin",
            "stdout",
            "--tessdata-dir",
            "tessdata",
            "-l",
            languages,
            "--dpi",
            "216",
            "-c",
            "tessedit_create_tsv=1",
        ])
        .env("OMP_THREAD_LIMIT", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
        .context("Could not start the bundled OCR engine")?;
    let mut input = child.stdin.take().context("OCR input pipe missing")?;
    let output = child.stdout.take().context("OCR output pipe missing")?;
    let errors = child.stderr.take().context("OCR error pipe missing")?;
    let (status, written, output, errors) = std::thread::scope(|scope| -> Result<_> {
        let writer = scope.spawn(move || input.write_all(png));
        let reader = scope.spawn(move || {
            let mut bytes = Vec::new();
            output
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let stderr = scope.spawn(move || {
            let mut bytes = Vec::new();
            errors
                .take(64 * 1024)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let started = Instant::now();
        let status = loop {
            if cancellation.is_cancelled() || started.elapsed() > Duration::from_secs(180) {
                let _ = child.kill();
                let _ = child.wait();
                cancellation.check()?;
                anyhow::bail!(
                    "Text recognition timed out on this page. Try a smaller page or lower-resolution scan."
                );
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
        };
        Ok((
            status,
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("OCR input worker failed"))?,
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("OCR output worker failed"))??,
            stderr
                .join()
                .map_err(|_| anyhow::anyhow!("OCR error worker failed"))??,
        ))
    })?;
    cancellation.check()?;
    ensure!(
        status.success(),
        "Text recognition failed: {}",
        String::from_utf8_lossy(&errors)
    );
    written.context("Could not send the scanned page to OCR")?;
    ensure!(
        output.len() <= 32 * 1024 * 1024,
        "OCR output exceeds the page limit"
    );
    parse_tsv(
        std::str::from_utf8(&output).context("OCR returned invalid UTF-8")?,
        pixels,
        points,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_words_by_page_block_paragraph_and_line_and_scales_coordinates() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t30\t60\t90\t30\t95\tTürkçe\n5\t1\t1\t1\t1\t2\t150\t60\t60\t30\t90\ttext\n5\t1\t2\t1\t1\t1\t30\t150\t90\t30\t96\tNext\n";
        let lines = parse_tsv(tsv, [600, 900], [200., 300.]).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Türkçe text");
        for (actual, expected) in lines[0].rect.into_iter().zip([10., 20., 70., 30.]) {
            assert!((actual - expected).abs() < 0.001);
        }
        assert_eq!(lines[1].text, "Next");
        assert!(parse_tsv(tsv, [100, 100], [200., 300.]).is_err());
        assert!(parse_tsv("not TSV", [600, 900], [200., 300.]).is_err());
    }
}
