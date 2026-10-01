use std::{
    path::Path,
    time::{Duration, Instant},
};

use bitcoin::Psbt;
use nokhwa::{
    Camera,
    pixel_format::LumaFormat,
    utils::{CameraIndex, RequestedFormat, RequestedFormatType},
};
use rxing::{RXingResult, RXingResultMetadataType, RXingResultMetadataValue};
use signer_lab_core::{CompactSeedQr, MnemonicSecret, QrScanProgress, QrScanner, StandardSeedQr};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct CameraPreview(Vec<u8>);

impl CameraPreview {
    pub(crate) fn into_bmp(self) -> Vec<u8> {
        self.0
    }
}

pub(crate) fn scan_psbt(
    control: &CancellationToken,
    on_preview: impl FnMut(CameraPreview),
) -> Result<Psbt, String> {
    let mut scanner = QrScanner::new();

    scan_code(control, on_preview, |detected| {
        let text = detected.getText();
        if !text.starts_with("B$") && !text.to_ascii_lowercase().starts_with("ur:") {
            return None;
        }

        match scanner.receive(text).map_err(error_message) {
            Ok(QrScanProgress::Complete { psbt, .. }) => Some(Ok(psbt)),
            Ok(QrScanProgress::InProgress { .. }) => None,
            Err(error) => Some(Err(error)),
        }
    })
}

pub(crate) fn scan_seed_qr(
    control: &CancellationToken,
    on_preview: impl FnMut(CameraPreview),
) -> Result<MnemonicSecret, String> {
    scan_code(control, on_preview, seed_qr_result)
}

pub(crate) fn read_seed_qr_image(path: &Path) -> Result<MnemonicSecret, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "SeedQR image path is not valid UTF-8".to_owned())?;
    let detected = rxing::helpers::detect_in_file(path, Some(rxing::BarcodeFormat::QR_CODE))
        .map_err(error_message)?;

    seed_qr_result(&detected)
        .unwrap_or_else(|| Err("QR does not contain standard or compact SeedQR data".to_owned()))
}

fn scan_code<T>(
    control: &CancellationToken,
    mut on_preview: impl FnMut(CameraPreview),
    mut decode: impl FnMut(&rxing::RXingResult) -> Option<Result<T, String>>,
) -> Result<T, String> {
    let requested =
        RequestedFormat::new::<LumaFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera = Camera::new(CameraIndex::Index(0), requested).map_err(error_message)?;

    camera.open_stream().map_err(error_message)?;
    let mut last_preview = None;

    while !control.is_cancelled() {
        let frame = camera.frame().map_err(error_message)?;
        let resolution = frame.resolution();
        let image = frame.decode_image::<LumaFormat>().map_err(error_message)?;
        let pixels = image.into_raw();
        if last_preview.is_none_or(|last_preview: Instant| {
            last_preview.elapsed() >= Duration::from_millis(100)
        }) {
            on_preview(CameraPreview(luma_bmp(
                &pixels,
                resolution.width(),
                resolution.height(),
                640,
            )));

            last_preview = Some(Instant::now());
        }

        let detected = rxing::helpers::detect_in_luma(
            pixels,
            resolution.width(),
            resolution.height(),
            Some(rxing::BarcodeFormat::QR_CODE),
        );

        let Ok(detected) = detected else {
            continue;
        };

        let Some(result) = decode(&detected) else {
            continue;
        };

        camera.stop_stream().map_err(error_message)?;
        if control.is_cancelled() {
            return Err("camera scan was cancelled".to_owned());
        }

        return result;
    }

    camera.stop_stream().map_err(error_message)?;
    Err("camera scan was cancelled".to_owned())
}

fn luma_bmp(pixels: &[u8], width: u32, height: u32, maximum_width: u32) -> Vec<u8> {
    let sample = width.div_ceil(maximum_width).max(1);
    let output_width = width / sample;
    let output_height = height / sample;
    let row_bytes = (output_width * 3).div_ceil(4) * 4;
    let pixel_bytes = row_bytes * output_height;
    let mut bmp = Vec::with_capacity((54 + pixel_bytes) as usize);

    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + pixel_bytes).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54_u32.to_le_bytes());
    bmp.extend_from_slice(&40_u32.to_le_bytes());
    bmp.extend_from_slice(&output_width.cast_signed().to_le_bytes());
    bmp.extend_from_slice(&output_height.cast_signed().to_le_bytes());
    bmp.extend_from_slice(&1_u16.to_le_bytes());
    bmp.extend_from_slice(&24_u16.to_le_bytes());
    bmp.extend_from_slice(&0_u32.to_le_bytes());
    bmp.extend_from_slice(&pixel_bytes.to_le_bytes());
    bmp.extend_from_slice(&2_835_u32.to_le_bytes());
    bmp.extend_from_slice(&2_835_u32.to_le_bytes());
    bmp.extend_from_slice(&0_u32.to_le_bytes());
    bmp.extend_from_slice(&0_u32.to_le_bytes());

    let padding = row_bytes - output_width * 3;
    for output_y in (0..output_height).rev() {
        let source_y = output_y * sample;
        for output_x in 0..output_width {
            let source_x = output_x * sample;
            let value = pixels[(source_y * width + source_x) as usize];
            bmp.extend_from_slice(&[value, value, value]);
        }

        bmp.extend(std::iter::repeat_n(0, padding as usize));
    }

    bmp
}

fn seed_qr_result(detected: &RXingResult) -> Option<Result<MnemonicSecret, String>> {
    let text = detected.getText().trim();
    if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        return seed_qr_payload(text, &[]);
    }

    let metadata = detected
        .getRXingResultMetadata()
        .get(&RXingResultMetadataType::BYTE_SEGMENTS);
    let Some(RXingResultMetadataValue::ByteSegments(segments)) = metadata else {
        return None;
    };

    let [entropy] = segments.as_slice() else {
        return None;
    };

    seed_qr_payload("", entropy)
}

fn seed_qr_payload(text: &str, raw_bytes: &[u8]) -> Option<Result<MnemonicSecret, String>> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(
            StandardSeedQr::parse(text)
                .map(StandardSeedQr::into_secret)
                .map_err(error_message),
        );
    }

    if !matches!(raw_bytes.len(), 16 | 20 | 24 | 28 | 32) {
        return None;
    }
    Some(
        CompactSeedQr::parse(raw_bytes)
            .map(CompactSeedQr::into_secret)
            .map_err(error_message),
    )
}

fn error_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{DevicePixels, Image, ImageFormat, TestAppContext, size};
    use rxing::{BarcodeFormat, Writer as _, qrcode::QRCodeWriter};

    use super::{luma_bmp, read_seed_qr_image, seed_qr_payload, seed_qr_result};

    const WORDS: &str = "forum undo fragile fade shy sign arrest garment culture tube off merit";
    const STANDARD: &str = "073318950739065415961602009907670428187212261116";
    const COMPACT: [u8; 16] = [
        0x5b, 0xbd, 0x9d, 0x71, 0xa8, 0xec, 0x79, 0x90, 0x83, 0x1a, 0xff, 0x35, 0x9d, 0x42, 0x65,
        0x45,
    ];

    #[test]
    fn detected_standard_and_compact_seed_qr_have_same_secret() {
        let standard = seed_qr_payload(STANDARD, STANDARD.as_bytes())
            .expect("standard SeedQR payload is recognized")
            .expect("standard SeedQR decodes");
        let compact = seed_qr_payload("", &COMPACT)
            .expect("compact SeedQR payload is recognized")
            .expect("compact SeedQR decodes");

        assert_eq!(standard.expose_phrase(), WORDS);
        assert_eq!(compact.expose_phrase(), WORDS);
    }

    #[test]
    fn unrelated_qr_payload_is_ignored_by_camera_scan() {
        assert!(seed_qr_payload("https://example.com", b"https://example.com").is_none());
    }

    #[test]
    fn compact_seed_qr_uses_qr_byte_segment_metadata() {
        let mut detected =
            rxing::RXingResult::new("", vec![0; 26], Vec::new(), BarcodeFormat::QR_CODE);
        detected.putMetadata(
            rxing::RXingResultMetadataType::BYTE_SEGMENTS,
            rxing::RXingResultMetadataValue::ByteSegments(vec![COMPACT.to_vec()]),
        );

        let secret = seed_qr_result(&detected)
            .expect("compact SeedQR payload is recognized")
            .expect("compact SeedQR payload is valid");

        assert_eq!(secret.expose_phrase(), WORDS);
    }

    #[test]
    fn standard_seed_qr_round_trips_through_an_image_file() {
        let path =
            std::env::temp_dir().join(format!("signer-lab-seedqr-{}.png", std::process::id()));
        let matrix = QRCodeWriter
            .encode(STANDARD, &BarcodeFormat::QR_CODE, 256, 256)
            .expect("test SeedQR renders");
        rxing::helpers::save_image(path.to_str().expect("test path is UTF-8"), &matrix)
            .expect("test SeedQR image saves");

        let secret = read_seed_qr_image(&path).expect("SeedQR image imports");

        assert_eq!(secret.expose_phrase(), WORDS);
        let _ = std::fs::remove_file(path);
    }

    #[gpui_kit::test]
    fn camera_preview_decodes_with_the_gpui_image_path(cx: &mut TestAppContext) {
        let image = Image::from_bytes(ImageFormat::Bmp, luma_bmp(&[0, 64, 128, 255], 2, 2, 640));
        let decoded = cx
            .update(|cx| image.to_image_data(cx.svg_renderer()))
            .expect("GPUI decodes the camera BMP");

        assert_eq!(decoded.size(0), size(DevicePixels(2), DevicePixels(2)));
        assert_eq!(
            decoded.as_bytes(0),
            Some(
                [
                    0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 255, 255, 255, 255
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn camera_preview_encodes_a_valid_size_bmp() {
        let bmp = luma_bmp(&[0, 64, 128, 255], 2, 2, 640);

        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(
                bmp[18..22]
                    .try_into()
                    .expect("BMP width header has four bytes"),
            ),
            2
        );

        assert_eq!(
            u32::from_le_bytes(
                bmp[22..26]
                    .try_into()
                    .expect("BMP height header has four bytes"),
            ),
            2
        );

        assert_eq!(bmp.len(), 70);
    }
}
