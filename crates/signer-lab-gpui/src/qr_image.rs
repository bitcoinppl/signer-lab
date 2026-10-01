use std::{fmt::Write as _, sync::Arc};

use gpui::{Image, ImageFormat};
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
