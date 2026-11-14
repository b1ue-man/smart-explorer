//! The pixels of a pointer whose image the platform only draws (plan
//! 2026-11-14, package P6; the Windows host draws its pointer with
//! `DrawIconEx`): drawn once onto black and once onto white, the two results
//! give colors and alpha of every pixel. A pixel that comes out brighter on
//! black than on white inverts what lies under it (the text pointer of
//! Windows); an overlay cannot invert the viewer's picture, so such a pixel
//! becomes black and the transparent pixels beside it white, which stays
//! visible on every background.

/// RGBA bytes with premultiplied colors from the BGRA renderings of one
/// pointer of `width` pixels per row onto black (`on_black`) and onto white
/// (`on_white`).
pub fn from_two_backgrounds(on_black: &[u8], on_white: &[u8], width: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(on_black.len());
    let mut inverting = Vec::new();
    let pixels = on_black.chunks_exact(4).zip(on_white.chunks_exact(4));
    for (index, (dark, light)) in pixels.enumerate() {
        if (0..3).any(|channel| dark[channel] > light[channel]) {
            inverting.push(index);
            rgba.extend_from_slice(&[0, 0, 0, 255]);
            continue;
        }
        // Onto black a pixel shows color times alpha, onto white that plus
        // white times (1 - alpha): the difference is 255 times (1 - alpha).
        let transparent = (0..3)
            .map(|channel| light[channel] - dark[channel])
            .max()
            .unwrap_or(0);
        let alpha = 255 - transparent;
        rgba.extend_from_slice(&[
            dark[2].min(alpha),
            dark[1].min(alpha),
            dark[0].min(alpha),
            alpha,
        ]);
    }
    if width == 0 {
        return rgba;
    }
    let count = rgba.len() / 4;
    for index in inverting {
        let column = index % width;
        let mut beside = Vec::with_capacity(4);
        if column > 0 {
            beside.push(index - 1);
        }
        if column + 1 < width {
            beside.push(index + 1);
        }
        if index >= width {
            beside.push(index - width);
        }
        beside.push(index + width);
        for at in beside.into_iter().filter(|at| *at < count) {
            if rgba[at * 4 + 3] == 0 {
                rgba[at * 4..at * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::from_two_backgrounds;

    /// BGRA bytes of one row of gray pixels.
    fn row(values: &[u8]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| [*value, *value, *value, 0])
            .collect()
    }

    #[test]
    fn verbindungen_task_zeiger_two_renderings_give_color_and_alpha() {
        // Transparent, opaque black, opaque white, half-transparent white.
        let rgba = from_two_backgrounds(&row(&[0, 0, 255, 128]), &row(&[255, 0, 255, 255]), 4);
        assert_eq!(
            rgba,
            [
                [0, 0, 0, 0],
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [128, 128, 128, 128]
            ]
            .concat()
        );
        // The channels keep their order: blue on the wire becomes the third byte.
        let blue = from_two_backgrounds(&[200, 0, 0, 0], &[200, 0, 0, 0], 1);
        assert_eq!(blue, [0, 0, 200, 255]);
    }

    #[test]
    fn verbindungen_task_zeiger_an_inverting_pointer_stays_visible_on_every_background() {
        // A 3 x 3 pointer whose middle column inverts (white on black, black
        // on white), as the text pointer of Windows does; the rest is clear.
        let on_black = [row(&[0, 255, 0]), row(&[0, 255, 0]), row(&[0, 255, 0])].concat();
        let on_white = [
            row(&[255, 0, 255]),
            row(&[255, 0, 255]),
            row(&[255, 0, 255]),
        ]
        .concat();
        let rgba = from_two_backgrounds(&on_black, &on_white, 3);
        let pixel = |x: usize, y: usize| &rgba[(y * 3 + x) * 4..(y * 3 + x) * 4 + 4];
        for y in 0..3 {
            assert_eq!(pixel(1, y), [0, 0, 0, 255], "the inverting pixel is black");
            assert_eq!(pixel(0, y), [255, 255, 255, 255], "white beside it");
            assert_eq!(pixel(2, y), [255, 255, 255, 255], "white beside it");
        }
        // Without inverting pixels nothing is outlined.
        let plain = from_two_backgrounds(&row(&[0, 0, 0]), &row(&[255, 0, 255]), 3);
        assert_eq!(&plain[0..4], [0, 0, 0, 0]);
        assert_eq!(&plain[8..12], [0, 0, 0, 0]);
    }
}
