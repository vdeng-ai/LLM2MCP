use eframe::egui::IconData;

const SIZE: usize = 64;

pub fn app_icon() -> IconData {
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];

    fill_rounded_rect(&mut rgba, 3, 3, 61, 61, 14, [10, 29, 72, 255]);

    // Neural-side speech bubble: cyan outline with three compact nodes.
    stroke_rounded_rect(&mut rgba, (10, 14, 30, 43), 8, 3, [53, 198, 255, 255]);
    draw_line(&mut rgba, 16, 43, 16, 49, 3, [53, 198, 255, 255]);
    draw_line(&mut rgba, 16, 49, 22, 43, 3, [53, 198, 255, 255]);
    fill_circle(&mut rgba, 16, 24, 3, [236, 248, 255, 255]);
    fill_circle(&mut rgba, 24, 23, 3, [236, 248, 255, 255]);
    fill_circle(&mut rgba, 21, 33, 3, [236, 248, 255, 255]);
    draw_line(&mut rgba, 17, 25, 23, 24, 2, [236, 248, 255, 255]);
    draw_line(&mut rgba, 23, 25, 21, 31, 2, [236, 248, 255, 255]);
    draw_line(&mut rgba, 20, 31, 17, 26, 2, [236, 248, 255, 255]);

    // Bridge arrow.
    draw_line(&mut rgba, 31, 31, 43, 31, 4, [53, 198, 255, 255]);
    draw_line(&mut rgba, 40, 27, 45, 31, 3, [53, 198, 255, 255]);
    draw_line(&mut rgba, 40, 35, 45, 31, 3, [53, 198, 255, 255]);

    // Minimal plug-side connector.
    fill_rounded_rect(&mut rgba, 44, 23, 54, 39, 4, [244, 248, 255, 255]);
    draw_line(&mut rgba, 54, 27, 59, 27, 3, [244, 248, 255, 255]);
    draw_line(&mut rgba, 54, 35, 59, 35, 3, [244, 248, 255, 255]);
    draw_line(&mut rgba, 43, 27, 40, 27, 3, [244, 248, 255, 255]);
    draw_line(&mut rgba, 43, 35, 40, 35, 3, [244, 248, 255, 255]);

    IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

fn pixel(data: &mut [u8], x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
        return;
    }
    let offset = (y as usize * SIZE + x as usize) * 4;
    data[offset..offset + 4].copy_from_slice(&color);
}

fn fill_circle(data: &mut [u8], cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
    let r2 = radius * radius;
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                pixel(data, x, y, color);
            }
        }
    }
}

fn fill_rounded_rect(
    data: &mut [u8],
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    color: [u8; 4],
) {
    for y in top..bottom {
        for x in left..right {
            let inner_x = x.clamp(left + radius, right - radius - 1);
            let inner_y = y.clamp(top + radius, bottom - radius - 1);
            let dx = x - inner_x;
            let dy = y - inner_y;
            if dx * dx + dy * dy <= radius * radius {
                pixel(data, x, y, color);
            }
        }
    }
}

fn stroke_rounded_rect(
    data: &mut [u8],
    rect: (i32, i32, i32, i32),
    radius: i32,
    width: i32,
    color: [u8; 4],
) {
    let (left, top, right, bottom) = rect;
    fill_rounded_rect(data, left, top, right, bottom, radius, color);
    fill_rounded_rect(
        data,
        left + width,
        top + width,
        right - width,
        bottom - width,
        (radius - width).max(1),
        [10, 29, 72, 255],
    );
}

fn draw_line(data: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, width: i32, color: [u8; 4]) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = dx.abs().max(dy.abs()).max(1);
    for step in 0..=steps {
        let x = x0 + dx * step / steps;
        let y = y0 + dy * step / steps;
        fill_circle(data, x, y, width.max(1) / 2, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_dimensions_and_visible_pixels() {
        let icon = app_icon();
        assert_eq!(icon.width, 64);
        assert_eq!(icon.height, 64);
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}
