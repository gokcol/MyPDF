//! Native Windows print dialog and GDI output, independent of PDF associations.
use anyhow::{Context as _, Result, bail, ensure};
use mypdf_milestone0::viewer::{self, PageSize};
use std::{ops::Range, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE,
        HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY,
        PHYSICALWIDTH, SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchDIBits, VERTRES,
    },
    Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage},
    System::Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    UI::Controls::Dialogs::{
        CommDlgExtendedError, DEVNAMES, PD_HIDEPRINTTOFILE, PD_NOSELECTION, PD_PAGENUMS,
        PD_RETURNDC, PD_RETURNDEFAULT, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
    },
};

struct PrinterDc(HDC);
// SAFETY: a printer DC is not a window's GetDC handle. Ownership is moved once
// from the UI thread to the print worker; it is never shared or used concurrently.
// https://learn.microsoft.com/windows/win32/procthread/multiple-threads-and-gdi-objects
unsafe impl Send for PrinterDc {}
impl Drop for PrinterDc {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the DC returned by PrintDlgW/CreateDCW.
        if !self.0.is_null() {
            unsafe { DeleteDC(self.0) };
        }
    }
}

pub(super) struct Job {
    dc: PrinterDc,
    pages: Range<usize>,
    settings: Settings,
}

#[derive(Default)]
struct Settings {
    mode: HGLOBAL,
    names: HGLOBAL,
}
// SAFETY: these uniquely owned global allocations move with the printer DC.
// They are only locked on the UI thread and freed once, after printing.
unsafe impl Send for Settings {}
impl Drop for Settings {
    fn drop(&mut self) {
        for handle in [self.mode, self.names] {
            if !handle.is_null() {
                unsafe { GlobalFree(handle) };
            }
        }
    }
}
impl Settings {
    fn printer_name(&self) -> String {
        // SAFETY: the dialog owns a DEVNAMES followed by UTF-16 strings. Bound
        // every read by the allocation size and release the lock afterwards.
        unsafe {
            let size = GlobalSize(self.names);
            let data = GlobalLock(self.names);
            if data.is_null() {
                return "Selected printer".into();
            }
            let name = if size >= std::mem::size_of::<DEVNAMES>() {
                let offset = (*data.cast::<DEVNAMES>()).wDeviceOffset as usize;
                let words = std::slice::from_raw_parts(data.cast::<u16>(), size / 2);
                words.get(offset..).map(|s| {
                    String::from_utf16_lossy(
                        &s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())],
                    )
                })
            } else {
                None
            };
            GlobalUnlock(self.names);
            name.filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Selected printer".into())
        }
    }
}

/// Printer metrics shared by preview and output. GDI's origin is the printable
/// area; the preview includes the physical paper and its unprintable margins.
#[derive(Clone, Copy)]
pub(super) struct Paper {
    area: [i32; 2],
    dpi: [i32; 2],
    physical: [i32; 2],
    offset: [i32; 2],
}
impl Paper {
    pub fn points(self) -> [f32; 2] {
        std::array::from_fn(|i| self.physical[i] as f32 * 72. / self.dpi[i] as f32)
    }
    pub fn page_rect(self, page: PageSize) -> Result<[f32; 4]> {
        let [x, y, w, h] = fit_page(page, self.area, self.dpi)?;
        Ok([
            (x + self.offset[0]) as f32 / self.physical[0] as f32,
            (y + self.offset[1]) as f32 / self.physical[1] as f32,
            w as f32 / self.physical[0] as f32,
            h as f32 / self.physical[1] as f32,
        ])
    }
}

fn dialog_options(owner: isize, count: usize) -> Result<PRINTDLGW> {
    ensure!(count > 0, "The document has no pages to print.");
    let count = u16::try_from(count).context("Print supports up to 65,535 pages per document.")?;
    Ok(PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        hwndOwner: owner as HWND,
        // The driver handles copies/collation exactly once. The raw spool-file
        // checkbox is hidden; virtual printers such as Print to PDF remain available.
        Flags: PD_RETURNDC | PD_USEDEVMODECOPIESANDCOLLATE | PD_NOSELECTION | PD_HIDEPRINTTOFILE,
        nMinPage: 1,
        nMaxPage: count,
        nFromPage: 1,
        nToPage: count,
        nCopies: 1,
        ..Default::default()
    })
}

fn selected_pages(dialog: &PRINTDLGW, count: usize) -> Result<Range<usize>> {
    if dialog.Flags & PD_PAGENUMS == 0 {
        return Ok(0..count);
    }
    let first = usize::from(dialog.nFromPage);
    let last = usize::from(dialog.nToPage);
    ensure!(
        first > 0 && first <= last && last <= count,
        "The selected page range is invalid."
    );
    Ok(first - 1..last)
}

pub(super) fn choose_printer(
    owner: isize,
    count: usize,
    mut previous: Option<&mut Job>,
    defaults: bool,
) -> Result<Option<Job>> {
    let mut dialog = dialog_options(owner, count)?;
    if defaults {
        dialog.Flags |= PD_RETURNDEFAULT;
    }
    if let Some(job) = previous.as_mut() {
        let settings = std::mem::take(&mut job.settings);
        dialog.hDevMode = settings.mode;
        dialog.hDevNames = settings.names;
        std::mem::forget(settings); // Ownership passes to the dialog, then back below.
        if job.pages != (0..count) {
            dialog.Flags |= PD_PAGENUMS;
            dialog.nFromPage = (job.pages.start + 1) as u16;
            dialog.nToPage = job.pages.end as u16;
        }
    }
    // SAFETY: called on the UI thread with a live owner (or null in native tests).
    // PrintDlgW initializes the owned handles; none escape without a Drop guard.
    let (accepted, error) = unsafe {
        let accepted = PrintDlgW(&mut dialog) != 0;
        let error = if accepted { 0 } else { CommDlgExtendedError() };
        (accepted, error)
    };
    let settings = Settings {
        mode: dialog.hDevMode,
        names: dialog.hDevNames,
    };
    let dc = PrinterDc(dialog.hDC);
    if !accepted {
        if let Some(job) = previous {
            job.settings = settings;
        }
        if error == 0 {
            return Ok(None);
        }
        bail!(
            "Windows could not open the print dialog (error 0x{error:04X}). Check that a printer is installed and available in Windows Settings."
        );
    }
    ensure!(!dc.0.is_null(), "Windows did not return a printer.");
    Ok(Some(Job {
        dc,
        pages: selected_pages(&dialog, count)?,
        settings,
    }))
}

/// Center within the printer's printable area, preserving physical proportions
/// even for devices with different horizontal and vertical resolutions.
fn fit_page(page: PageSize, area: [i32; 2], dpi: [i32; 2]) -> Result<[i32; 4]> {
    ensure!(
        page.width.is_finite() && page.height.is_finite() && page.width > 0.0 && page.height > 0.0,
        "The page has invalid dimensions."
    );
    ensure!(
        area.iter().chain(dpi.iter()).all(|v| *v > 0),
        "The printer returned invalid paper dimensions."
    );
    let width = f64::from(page.width) * f64::from(dpi[0]) / 72.0;
    let height = f64::from(page.height) * f64::from(dpi[1]) / 72.0;
    let scale = (f64::from(area[0]) / width)
        .min(f64::from(area[1]) / height)
        .min(1.0);
    let width = (width * scale).round().clamp(1.0, f64::from(area[0])) as i32;
    let height = (height * scale).round().clamp(1.0, f64::from(area[1])) as i32;
    Ok([(area[0] - width) / 2, (area[1] - height) / 2, width, height])
}

fn bitmap_info(size: [usize; 2]) -> Result<BITMAPINFO> {
    let width = i32::try_from(size[0])?;
    let height = i32::try_from(size[1])?;
    ensure!(width > 0 && height > 0, "The rendered page is empty.");
    Ok(BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // Top-down rows, matching the PDF renderer.
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    })
}

fn draw_page(dc: HDC, size: [usize; 2], rgba: &mut [u8], target: [i32; 4]) -> Result<()> {
    let info = bitmap_info(size)?;
    ensure!(
        size[0].checked_mul(size[1]).and_then(|v| v.checked_mul(4)) == Some(rgba.len()),
        "The rendered page is incomplete."
    );
    // The renderer composites on white. GDI's 32-bit BI_RGB requires BGRX.
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0;
    }
    let [left, top, width, height] = target;
    // SAFETY: the DC is uniquely owned, and the bitmap describes the checked
    // pixel buffer exactly. The buffer remains alive until GDI finishes copying.
    let lines = unsafe {
        SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, ptr::null_mut());
        StretchDIBits(
            dc,
            left,
            top,
            width,
            height,
            0,
            0,
            size[0] as i32,
            size[1] as i32,
            rgba.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    ensure!(
        lines > 0,
        "The printer could not draw the page: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

struct ActiveDocument<'a>(&'a PrinterDc, bool);
impl Drop for ActiveDocument<'_> {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: abort only this job if rendering/spooling failed or panicked.
            unsafe { AbortDoc(self.0.0) };
        }
    }
}

impl Job {
    pub(super) fn pages(&self) -> Range<usize> {
        self.pages.clone()
    }
    pub(super) fn printer_name(&self) -> String {
        self.settings.printer_name()
    }
    pub(super) fn paper(&self) -> Result<Paper> {
        let dc = self.dc.0;
        // SAFETY: a live, exclusively owned printer DC.
        let paper = unsafe {
            Paper {
                area: [
                    GetDeviceCaps(dc, HORZRES as i32),
                    GetDeviceCaps(dc, VERTRES as i32),
                ],
                dpi: [
                    GetDeviceCaps(dc, LOGPIXELSX as i32),
                    GetDeviceCaps(dc, LOGPIXELSY as i32),
                ],
                physical: [
                    GetDeviceCaps(dc, PHYSICALWIDTH as i32),
                    GetDeviceCaps(dc, PHYSICALHEIGHT as i32),
                ],
                offset: [
                    GetDeviceCaps(dc, PHYSICALOFFSETX as i32),
                    GetDeviceCaps(dc, PHYSICALOFFSETY as i32),
                ],
            }
        };
        ensure!(
            paper
                .area
                .iter()
                .chain(paper.dpi.iter())
                .chain(paper.physical.iter())
                .all(|v| *v > 0)
                && paper.offset.iter().all(|v| *v >= 0),
            "The printer returned invalid paper dimensions."
        );
        Ok(paper)
    }
    pub(super) fn spool(
        self,
        bytes: &[u8],
        sizes: &[PageSize],
        name: &str,
        output: Option<&Path>,
    ) -> Result<()> {
        ensure!(
            !self.pages.is_empty() && self.pages.end <= sizes.len(),
            "The selected pages are unavailable."
        );
        let name: Vec<u16> = name
            .chars()
            .filter(|c| *c != '\0')
            .collect::<String>()
            .encode_utf16()
            .chain([0])
            .collect();
        use std::os::windows::ffi::OsStrExt;
        let output = output.map(|path| {
            path.as_os_str()
                .encode_wide()
                .chain([0])
                .collect::<Vec<_>>()
        });
        let dc = self.dc.0;
        let Paper { area, dpi, .. } = self.paper()?;
        let targets = self
            .pages
            .clone()
            .map(|page| fit_page(sizes[page], area, dpi))
            .collect::<Result<Vec<_>>>()?;
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: name.as_ptr(),
            lpszOutput: output.as_ref().map_or(ptr::null(), |path| path.as_ptr()),
            ..Default::default()
        };
        // SAFETY: valid DC and terminated strings. This blocking operation runs
        // on the dedicated print worker, never on the UI thread.
        ensure!(
            unsafe { StartDocW(dc, &info) } > 0,
            "Windows could not start the print job: {}",
            std::io::Error::last_os_error()
        );
        let mut document = ActiveDocument(&self.dc, true);
        for (page, target) in self.pages.clone().zip(targets) {
            // Bound raster memory using the same renderer limits as the viewer.
            // Up to 300 dpi at the physical printed size, one page at a time.
            let scale =
                (target[2] as f32 / dpi[0] as f32 * 300.0 / sizes[page].width).min(300.0 / 72.0);
            let (size, mut pixels) = viewer::render_page(bytes, page, scale)
                .with_context(|| format!("Could not render page {}", page + 1))?;
            ensure!(
                unsafe { StartPage(dc) } > 0,
                "The printer could not start page {}: {}",
                page + 1,
                std::io::Error::last_os_error()
            );
            draw_page(dc, size, &mut pixels, target)
                .with_context(|| format!("Could not print page {}", page + 1))?;
            ensure!(
                unsafe { EndPage(dc) } > 0,
                "The printer could not finish page {}: {}",
                page + 1,
                std::io::Error::last_os_error()
            );
        }
        ensure!(
            unsafe { EndDoc(dc) } > 0,
            "Windows could not finish the print job: {}",
            std::io::Error::last_os_error()
        );
        document.1 = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_dialog_uses_native_output_and_honors_all_or_selected_pages() {
        let mut options = dialog_options(0, 8).unwrap();
        assert_eq!(options.Flags & PD_RETURNDC, PD_RETURNDC);
        assert_eq!(
            options.Flags & PD_USEDEVMODECOPIESANDCOLLATE,
            PD_USEDEVMODECOPIESANDCOLLATE
        );
        assert_eq!(selected_pages(&options, 8).unwrap(), 0..8);
        options.Flags |= PD_PAGENUMS;
        options.nFromPage = 3;
        options.nToPage = 5;
        assert_eq!(selected_pages(&options, 8).unwrap(), 2..5);
        options.nToPage = 9;
        assert!(selected_pages(&options, 8).is_err());
        options.nToPage = 2;
        assert!(selected_pages(&options, 8).is_err());
        assert!(dialog_options(0, 0).is_err());
        assert!(dialog_options(0, 65_536).is_err());
    }

    #[test]
    fn print_layout_fits_portrait_landscape_and_asymmetric_printer_resolutions() {
        let portrait = PageSize {
            width: 612.0,
            height: 792.0,
        };
        assert_eq!(
            fit_page(portrait, [2550, 3300], [300, 300]).unwrap(),
            [0, 0, 2550, 3300]
        );
        assert_eq!(
            fit_page(portrait, [2550, 6600], [300, 600]).unwrap(),
            [0, 0, 2550, 6600]
        );
        let landscape = PageSize {
            width: 792.0,
            height: 612.0,
        };
        let [x, y, width, height] = fit_page(landscape, [2400, 3150], [300, 300]).unwrap();
        assert_eq!((x, width), (0, 2400));
        assert!(y > 0 && height < 3150);
        assert_eq!(
            fit_page(
                PageSize {
                    width: 72.0,
                    height: 72.0
                },
                [2400, 3150],
                [300, 300]
            )
            .unwrap(),
            [1050, 1425, 300, 300]
        );
        assert!(fit_page(portrait, [0, 100], [300, 300]).is_err());
    }

    #[test]
    fn preview_includes_physical_margins_and_matches_the_printed_rectangle() {
        let paper = Paper {
            area: [2400, 6300],
            dpi: [300, 600],
            physical: [2550, 6600],
            offset: [50, 100],
        };
        assert_eq!(paper.points(), [612., 792.]);
        for page in [
            PageSize {
                width: 792.,
                height: 612.,
            },
            PageSize {
                width: 72.,
                height: 72.,
            },
        ] {
            let preview = paper.page_rect(page).unwrap();
            let printed = fit_page(page, paper.area, paper.dpi).unwrap();
            for (i, expected) in [printed[0] + 50, printed[1] + 100, printed[2], printed[3]]
                .into_iter()
                .enumerate()
            {
                assert!((preview[i] * paper.physical[i % 2] as f32 - expected as f32).abs() < 0.01);
            }
            assert!(preview[0] > 0. && preview[1] > 0.);
            assert!(preview[0] + preview[2] <= 1. && preview[1] + preview[3] <= 1.);
        }
    }

    #[test]
    fn gdi_print_bitmap_preserves_colors_and_top_down_orientation() {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteObject, SelectObject,
        };
        let info = bitmap_info([2, 2]).unwrap();
        let mut bits = ptr::null_mut();
        // SAFETY: the test owns its memory DC and bitmap, never a physical printer.
        unsafe {
            let dc = PrinterDc(CreateCompatibleDC(ptr::null_mut()));
            assert!(!dc.0.is_null());
            let bitmap =
                CreateDIBSection(dc.0, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            assert!(!bitmap.is_null() && !bits.is_null());
            let previous = SelectObject(dc.0, bitmap);
            let mut rgba = vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ];
            let result = draw_page(dc.0, [2, 2], &mut rgba, [0, 0, 2, 2]);
            let actual = std::slice::from_raw_parts(bits.cast::<u8>(), 16).to_vec();
            SelectObject(dc.0, previous);
            DeleteObject(bitmap);
            result.unwrap();
            assert_eq!(&actual[0..3], &[0, 0, 255], "red stays at top left");
            assert_eq!(&actual[8..11], &[255, 0, 0], "blue stays at bottom left");
        }
    }

    #[test]
    #[ignore = "Requires the Windows Microsoft Print to PDF virtual printer; writes only a temporary PDF"]
    fn windows_virtual_printer_outputs_the_selected_page() {
        use windows_sys::{Win32::Graphics::Gdi::CreateDCW, core::w};
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("printed.pdf");
        let original = mypdf_milestone0::convert::blank(144., 144.).unwrap();
        let bytes = mypdf_milestone0::pages::apply(
            &original,
            &mypdf_milestone0::pages::Operation::InsertBlank {
                at: 1,
                width: 144.,
                height: 72.,
            },
        )
        .unwrap();
        use mypdf_milestone0::annotations::{Annotation, Kind, Style, export};
        let mark = |page, rect, color| {
            Annotation::new(
                page,
                rect,
                Kind::Rectangle,
                Style {
                    color,
                    fill: Some(color),
                    ..Default::default()
                },
            )
        };
        let bytes = export(
            &bytes,
            &[
                mark(0, [10., 10., 40., 30.], [0., 0., 1.]),
                mark(1, [10., 10., 40., 30.], [1., 0., 0.]),
                mark(1, [90., 40., 120., 60.], [0., 1., 0.]),
            ],
        )
        .unwrap();
        // Only this named virtual printer is used, never the default printer.
        let dc = unsafe {
            CreateDCW(
                w!("WINSPOOL"),
                w!("Microsoft Print to PDF"),
                ptr::null(),
                ptr::null(),
            )
        };
        assert!(
            !dc.is_null(),
            "Microsoft Print to PDF is unavailable: {}",
            std::io::Error::last_os_error()
        );
        let job = Job {
            dc: PrinterDc(dc),
            pages: 1..2,
            settings: Settings::default(),
        };
        let sizes = vec![
            PageSize {
                width: 144.,
                height: 144.,
            },
            PageSize {
                width: 144.,
                height: 72.,
            },
        ];
        // Opening and rendering the preview must not send a job. It uses the
        // same annotated snapshot and selected range that is spooled below.
        let preview = super::super::preview::Preview::new(
            bytes.clone(),
            sizes.clone(),
            "Regression".into(),
            job,
        )
        .unwrap();
        assert!(!output.exists());
        assert_eq!(preview.job.pages(), 1..2);
        preview
            .job
            .spool(
                &preview.bytes,
                &preview.pages,
                "MyPDF print regression",
                Some(&output),
            )
            .unwrap();
        let printed = lopdf::Document::load(&output).unwrap();
        assert_eq!(
            printed.get_pages().len(),
            1,
            "only the requested page is printed"
        );
        let printed_bytes = std::fs::read(&output).unwrap();
        let (_, rgba) = viewer::render_page(&printed_bytes, 0, 1.0).unwrap();
        let red = rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 180 && p[1] < 80 && p[2] < 80)
            .count();
        let green = rgba
            .chunks_exact(4)
            .filter(|p| p[1] > 180 && p[0] < 80 && p[2] < 80)
            .count();
        let blue = rgba
            .chunks_exact(4)
            .filter(|p| p[2] > 180 && p[0] < 80 && p[1] < 80)
            .count();
        assert!(
            red > 100 && green > 100,
            "printed annotations keep their colors: red={red}, green={green}"
        );
        assert_eq!(blue, 0, "the unselected first page is not printed");
    }
}
