use std::{fmt::Write as _, sync::Arc};

use gpui_kit::{Image, ImageFormat};
use rxing::{BarcodeFormat, Writer as _, qrcode::QRCodeWriter};

pub(crate) fn render(frame: &str, side: u32) -> Result<Arc<Image>, String> {
    let matrix = matrix(frame)?;
    let svg = matrix_svg(&matrix, side)?;

    Ok(Arc::new(Image::from_bytes(ImageFormat::Svg, svg)))
}

pub(crate) fn svg(frame: &str) -> Result<Vec<u8>, String> {
    let matrix = matrix(frame)?;
    let side = matrix.getWidth() * 8;

    matrix_svg(&matrix, side)
}

fn matrix(frame: &str) -> Result<rxing::common::BitMatrix, String> {
    QRCodeWriter
        .encode(frame, &BarcodeFormat::QR_CODE, 0, 0)
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
mod tests {
    use gpui_kit::{DevicePixels, TestAppContext, size};

    use super::render;

    #[gpui_kit::test]
    fn qr_image_decodes_with_the_smooth_svg_scale(cx: &mut TestAppContext) {
        let image = render("signer-lab image transport", 260).expect("QR image renders");
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
