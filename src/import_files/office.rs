//! Use locally installed office applications. Never download a converter.
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const CONVERTER_WARNING: &str = "DOCX and RTF import requires Microsoft Word or LibreOffice. Install one, or export the document as PDF and insert that file.";

fn libreoffice() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"),
        PathBuf::from("/usr/bin/libreoffice"),
        PathBuf::from("/usr/bin/soffice"),
    ];
    for root in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(root) {
            candidates.push(PathBuf::from(root).join("LibreOffice/program/soffice.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}
fn has_word() -> bool {
    #[cfg(target_os = "macos")]
    {
        Path::new("/Applications/Microsoft Word.app").is_dir()
    }
    #[cfg(target_os = "windows")]
    {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .any(|root| {
                [
                    "Microsoft Office/root/Office16/WINWORD.EXE",
                    "Microsoft Office/Office16/WINWORD.EXE",
                    "Microsoft Office/Office15/WINWORD.EXE",
                ]
                .iter()
                .any(|suffix| Path::new(&root).join(suffix).is_file())
            })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}
pub fn converter_available() -> bool {
    has_word() || libreoffice().is_some()
}

pub(super) fn run(command: &mut Command) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .context("Could not start the installed converter")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "The installed converter could not export this file to PDF"
            );
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "Conversion timed out. Export the document as PDF in Word or LibreOffice, then insert that PDF."
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn convert(bytes: &[u8], extension: &str) -> Result<Vec<u8>> {
    ensure!(
        matches!(extension, "docx" | "rtf"),
        "Unsupported office document"
    );
    ensure!(converter_available(), "{CONVERTER_WARNING}");
    let temp = tempfile::tempdir()?;
    let input = temp.path().join(format!("input.{extension}"));
    let output = temp.path().join("input.pdf");
    std::fs::write(&input, bytes)?;
    let mut errors = Vec::new();
    if has_word() {
        match word(&input, &output).and_then(|()| read_pdf(&output)) {
            Ok(pdf) => return Ok(pdf),
            Err(e) => errors.push(format!("Microsoft Word: {e:#}")),
        }
        let _ = std::fs::remove_file(&output);
    }
    if let Some(executable) = libreoffice() {
        match with_libreoffice(&executable, &input, &output).and_then(|()| read_pdf(&output)) {
            Ok(pdf) => return Ok(pdf),
            Err(e) => errors.push(format!("LibreOffice: {e:#}")),
        }
    }
    anyhow::bail!(
        "{}\nExport the document as PDF and insert that file.",
        errors.join("\n")
    );
}
fn read_pdf(path: &Path) -> Result<Vec<u8>> {
    let bytes =
        crate::convert::read_bounded(path, 256 * 1024 * 1024).context("No PDF was produced")?;
    let doc = lopdf::Document::load_mem(&bytes).context("The converter produced an invalid PDF")?;
    ensure!(
        !doc.is_encrypted() && !doc.get_pages().is_empty(),
        "The converted PDF has no accessible pages"
    );
    Ok(bytes)
}
fn with_libreoffice(executable: &Path, input: &Path, output: &Path) -> Result<()> {
    let profile = tempfile::tempdir()?;
    std::fs::create_dir(profile.path().join("user"))?;
    // A private profile avoids changing the user's settings or using their open instance.
    std::fs::write(
        profile.path().join("user/registrymodifications.xcu"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry"><item oor:path="/org.openoffice.Office.Common/Security/Scripting">
<prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop>
<prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop>
<prop oor:name="DisableActiveContent" oor:op="fuse"><value>true</value></prop>
</item></oor:items>"#,
    )?;
    let uri = reqwest::Url::from_directory_path(profile.path())
        .map_err(|_| anyhow::anyhow!("Invalid converter profile path"))?;
    run(Command::new(executable)
        .arg(format!("-env:UserInstallation={uri}"))
        .args([
            "--headless",
            "--nologo",
            "--nodefault",
            "--norestore",
            "--convert-to",
            "pdf:writer_pdf_Export",
            "--outdir",
        ])
        .arg(output.parent().context("Missing output folder")?)
        .arg(input))
}
#[cfg(target_os = "macos")]
fn word(input: &Path, output: &Path) -> Result<()> {
    run(Command::new("/usr/bin/osascript")
        .args(["-e", include_str!("word.applescript")])
        .arg(input)
        .arg(output))
    .context("Word must be licensed and allowed to receive automation requests from MyPDF")
}
#[cfg(target_os = "windows")]
fn word(input: &Path, output: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let script = temp.path().join("export.ps1");
    std::fs::write(&script, include_str!("word.ps1"))?;
    let powershell =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    run(Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-InputPath")
        .arg(input)
        .arg("-OutputPath")
        .arg(output))
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn word(_: &Path, _: &Path) -> Result<()> {
    anyhow::bail!("Microsoft Word is unavailable on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a local LibreOffice installation"]
    fn installed_libreoffice_converts_rtf_layout() {
        let executable = libreoffice().expect("LibreOffice installed");
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.rtf");
        let output = temp.path().join("input.pdf");
        std::fs::write(
            &input,
            br"{\rtf1\ansi\paperw11906\paperh16838\b Heading\b0\par First page\page Second page}",
        )
        .unwrap();
        with_libreoffice(&executable, &input, &output).unwrap();
        let pdf = lopdf::Document::load_mem(&read_pdf(&output).unwrap()).unwrap();
        assert_eq!(pdf.get_pages().len(), 2);
        assert!(pdf.extract_text(&[1]).unwrap().contains("Heading"));
        assert!(pdf.extract_text(&[2]).unwrap().contains("Second page"));
    }
    #[test]
    #[ignore = "requires a local LibreOffice installation"]
    fn installed_libreoffice_converts_docx_table_and_page_break() {
        use std::io::{Cursor, Write};
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t>Document heading</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="7000" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="8"/><w:bottom w:val="single" w:sz="8"/><w:insideV w:val="single" w:sz="8"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="3500"/><w:gridCol w:w="3500"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Left cell</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Right cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:br w:type="page"/><w:t>Second page</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr></w:body></w:document>"#,
            ),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(xml.as_bytes()).unwrap();
        }
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.docx");
        let output = temp.path().join("input.pdf");
        std::fs::write(&input, archive.finish().unwrap().into_inner()).unwrap();
        with_libreoffice(&libreoffice().unwrap(), &input, &output).unwrap();
        let pdf = lopdf::Document::load_mem(&read_pdf(&output).unwrap()).unwrap();
        assert_eq!(pdf.get_pages().len(), 2);
        let first = pdf.extract_text(&[1]).unwrap();
        assert!(
            first.contains("Left cell")
                && first.contains("Right cell")
                && first.contains("Document heading")
        );
        assert!(pdf.extract_text(&[2]).unwrap().contains("Second page"));
    }
}
