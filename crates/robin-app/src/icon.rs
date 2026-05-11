#[cfg(not(target_os = "windows"))]
pub fn tray_icon(png_data: &[u8]) -> Vec<u8> {
    png_data.to_vec()
}

#[cfg(target_os = "windows")]
pub fn tray_icon(png_data: &[u8]) -> Vec<u8> {
    let img = match image::load_from_memory_with_format(png_data, image::ImageFormat::Png) {
        Ok(i) => i,
        Err(_) => return png_data.to_vec(),
    };
    let icon_w = if img.width() >= 256 { 0u8 } else { img.width() as u8 };
    let icon_h = if img.height() >= 256 { 0u8 } else { img.height() as u8 };
    let mut buf = Vec::new();
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.push(icon_w);
    buf.push(icon_h);
    buf.push(0);
    buf.push(0);
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&(png_data.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(6u32 + 16u32).to_le_bytes());
    buf.extend_from_slice(png_data);
    buf
}