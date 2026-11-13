use anyhow::{Context, Result};
use image::RgbaImage;

#[cfg(target_os = "macos")]
pub(super) fn decode(bytes: &[u8], extension: &str) -> Result<RgbaImage> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join(format!("input.{extension}"));
    let output = temp.path().join("output.png");
    std::fs::write(&input, bytes)?;
    super::office::run(
        std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(&input)
            .arg("--out")
            .arg(&output),
    )
    .context("macOS could not decode this HEIC/HEIF image")?;
    crate::content::decode_image(&crate::convert::read_bounded(&output, 32 * 1024 * 1024)?)
}
#[cfg(target_os = "windows")]
pub(super) fn decode(bytes: &[u8], extension: &str) -> Result<RgbaImage> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*, System::Com::*},
        core::PCWSTR,
    };
    let temp = tempfile::tempdir()?;
    let input = temp.path().join(format!("input.{extension}"));
    std::fs::write(&input, bytes)?;
    let path: Vec<u16> = input.as_os_str().encode_wide().chain(Some(0)).collect();
    // Called on a dedicated import worker; COM objects stay on this thread.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(path.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnLoad)
            .context("HEIC/HEIF import requires the Windows HEIF image codec. Install the codec or convert this image to PNG or JPEG.")?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut w, mut h) = (0, 0);
        converter.GetSize(&mut w, &mut h)?;
        super::dimensions(w, h)?;
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        RgbaImage::from_raw(w, h, pixels).context("Invalid decoded image")
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn decode(_: &[u8], _: &str) -> Result<RgbaImage> {
    anyhow::bail!(
        "HEIC/HEIF conversion requires macOS or the Windows HEIF codec. Convert this image to PNG or JPEG."
    )
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires macOS HEIC encoder"]
    fn macos_heic_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("source.png");
        let heic = temp.path().join("source.heic");
        image::RgbImage::from_pixel(80, 40, image::Rgb([200, 30, 20]))
            .save(&input)
            .unwrap();
        super::super::office::run(
            std::process::Command::new("/usr/bin/sips")
                .args(["-s", "format", "heic"])
                .arg(&input)
                .arg("--out")
                .arg(&heic),
        )
        .unwrap();
        let decoded = decode(&std::fs::read(heic).unwrap(), "heic").unwrap();
        assert_eq!(decoded.dimensions(), (80, 40));
        let p = decoded.get_pixel(40, 20);
        assert!(p[0] > 150 && p[1] < 80);
    }
}
