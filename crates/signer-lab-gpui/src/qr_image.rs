use std::{fmt::Write as _, sync::Arc};

use gpui_kit::{Image, ImageFormat};
use rxing::{BarcodeFormat, EncodeHintValue, EncodeHints, Writer as _, qrcode::QRCodeWriter};
use signer_lab_core::QrErrorCorrection;

pub(crate) fn render(
    frame: &str,
    error_correction: QrErrorCorrection,
    side: u32,
) -> Result<Arc<Image>, String> {
    let matrix = matrix(frame, error_correction)?;
    let svg = matrix_svg(&matrix, side)?;

    Ok(Arc::new(Image::from_bytes(ImageFormat::Svg, svg)))
}

pub(crate) fn svg(frame: &str, error_correction: QrErrorCorrection) -> Result<Vec<u8>, String> {
    let matrix = matrix(frame, error_correction)?;
    let side = matrix.getWidth() * 8;

    matrix_svg(&matrix, side)
}

fn matrix(
    frame: &str,
    error_correction: QrErrorCorrection,
) -> Result<rxing::common::BitMatrix, String> {
    // rxing parses the level from its ISO 18004 letter
    let level = match error_correction {
        QrErrorCorrection::Low => "L",
        QrErrorCorrection::Medium => "M",
    };
    let hints = EncodeHints::default().with(EncodeHintValue::ErrorCorrection(level.to_owned()));

    QRCodeWriter
        .encode_with_hints(frame, &BarcodeFormat::QR_CODE, 0, 0, &hints)
        .map_err(error_message)
}

fn matrix_svg(matrix: &rxing::common::BitMatrix, side: u32) -> Result<Vec<u8>, String> {
    let width = matrix.getWidth();
    let height = matrix.getHeight();
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{side}" height="{side}" viewBox="0 0 {width} {height}" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="white"/>"#
    );

    for y in 0..height {
        for x in 0..width {
            if matrix.get(x, y) {
                write!(
                    svg,
                    r#"<rect x="{x}" y="{y}" width="1" height="1" fill="black"/>"#
                )
                .map_err(error_message)?;
            }
        }
    }

    svg.push_str("</svg>");

    Ok(svg.into_bytes())
}

fn error_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use bitcoin::{
        OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, Witness, absolute::LockTime,
        psbt::raw, transaction::Version,
    };
    use gpui_kit::{DevicePixels, TestAppContext, size};
    use signer_lab_core::{QrDensity, QrErrorCorrection, QrFrames, QrTransport};

    use super::{matrix, render};

    /// UR frames for a PSBT that fills several default-length fragments
    pub(crate) fn multipart_ur_frames() -> QrFrames {
        let mut psbt = Psbt::from_unsigned_tx(Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: Vec::new(),
        })
        .expect("unsigned transaction is a valid PSBT");

        // an unassigned key type stands in for signatures and derivations
        psbt.inputs[0].unknown.insert(
            raw::Key {
                type_value: 0xf0,
                key: Vec::new(),
            },
            vec![7; 1_000],
        );

        QrFrames::from_psbt(&psbt, QrTransport::Ur, QrDensity::default())
            .expect("PSBT encodes as UR")
    }

    #[test]
    fn ur_symbols_are_smaller_than_lowercase_byte_mode_at_level_l() {
        let frames = multipart_ur_frames();
        let frame = &frames.frames()[0];

        let shown = matrix(frame, frames.error_correction()).expect("UR frame renders");
        let lowercase =
            matrix(&frame.to_ascii_lowercase(), QrErrorCorrection::Low).expect("UR frame renders");

        assert_eq!(frames.error_correction(), QrErrorCorrection::Medium);
        assert!(shown.getWidth() < lowercase.getWidth());
    }

    #[gpui_kit::test]
    fn qr_image_decodes_with_the_smooth_svg_scale(cx: &mut TestAppContext) {
        let image = render("signer-lab image transport", QrErrorCorrection::Low, 260)
            .expect("QR image renders");
        let decoded = cx
            .update(|cx| image.to_image_data(cx.svg_renderer()))
            .expect("GPUI decodes the QR SVG");

        // GPUI rasterizes SVGs at twice their logical size for smooth edges
        assert_eq!(decoded.size(0), size(DevicePixels(520), DevicePixels(520)));
        let pixels = decoded.as_bytes(0).expect("decoded QR has pixels");
        let pixels = pixels.as_chunks::<4>().0;
        assert!(pixels.contains(&[0, 0, 0, 255]));
        assert!(pixels.contains(&[255; 4]));
    }
}
