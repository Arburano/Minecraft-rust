//! RGBA8 images and mipmap generation.

/// A straight-alpha RGBA8 image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 4]>,
}

impl Rgba8Image {
    /// An image filled with one color.
    #[must_use]
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; (width * height) as usize],
        }
    }

    /// An image from pixels; `None` if the count does not match.
    #[must_use]
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Option<Self> {
        (pixels.len() == (width * height) as usize).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel at `(x, y)`.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Sets the pixel at `(x, y)`.
    pub fn set(&mut self, x: u32, y: u32, color: [u8; 4]) {
        let i = (y * self.width + x) as usize;
        self.pixels[i] = color;
    }

    /// Pixels row by row.
    #[must_use]
    pub fn pixels(&self) -> &[[u8; 4]] {
        &self.pixels
    }

    /// Raw RGBA bytes row by row.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.pixels.iter().flatten().copied().collect()
    }

    /// The next smaller mip level: 2×2 box filter, color weighted by alpha so
    /// transparent texels (leaves, glass) do not darken the edges.
    #[must_use]
    pub fn downsample(&self) -> Self {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut out = Self::filled(w, h, [0; 4]);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 3];
                let mut alpha = 0u32;
                let mut n = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (
                        (x * 2 + dx).min(self.width - 1),
                        (y * 2 + dy).min(self.height - 1),
                    );
                    let p = self.get(sx, sy);
                    let a = u32::from(p[3]);
                    for c in 0..3 {
                        sum[c] += u32::from(p[c]) * a;
                    }
                    alpha += a;
                    n += 1;
                }
                let color = match std::num::NonZeroU32::new(alpha) {
                    None => [0, 0, 0, 0],
                    Some(total) => {
                        let avg = |v: u32| ((v + alpha / 2) / total) as u8;
                        [
                            avg(sum[0]),
                            avg(sum[1]),
                            avg(sum[2]),
                            ((alpha + n / 2) / n) as u8,
                        ]
                    }
                };
                out.set(x, y, color);
            }
        }
        out
    }

    /// The full mip chain down to 1×1, level 0 first.
    #[must_use]
    pub fn mip_chain(&self) -> Vec<Self> {
        let mut chain = vec![self.clone()];
        while let Some(last) = chain.last() {
            if last.width == 1 && last.height == 1 {
                break;
            }
            let next = last.downsample();
            chain.push(next);
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_sizes_and_alpha_weighting() {
        let mut img = Rgba8Image::filled(16, 16, [255, 0, 0, 255]);
        img.set(0, 0, [0, 0, 255, 0]); // transparent blue must not tint
        let chain = img.mip_chain();
        let sizes: Vec<u32> = chain.iter().map(Rgba8Image::width).collect();
        assert_eq!(sizes, vec![16, 8, 4, 2, 1]);
        let p = chain[1].get(0, 0);
        assert_eq!(&p[..3], &[255, 0, 0]);
        assert_eq!(p[3], 191, "3 of 4 opaque texels, rounded");
        let empty = Rgba8Image::filled(2, 2, [9, 9, 9, 0]).downsample();
        assert_eq!(empty.get(0, 0), [0, 0, 0, 0]);
        assert!(Rgba8Image::from_pixels(2, 2, vec![[0; 4]; 3]).is_none());
    }
}
