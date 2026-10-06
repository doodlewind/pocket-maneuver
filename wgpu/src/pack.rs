//! Reading the pack: the PS Vita's (`profiles/vita60.json`), whole, a range
//! at a time so the interface can say how much has arrived, and its atlas
//! decoded from BC1 blocks to texels every WebGPU device samples.

use std::cell::Cell;
use std::rc::Rc;

use pocket_web_wgpu::source::Source;

/// Bytes asked for in one read. A pack cut into pieces fetches the pieces of a read all at once.
const READ: u64 = 8 << 20;

/// How much of the pack has arrived, for the loading screen.
#[derive(Clone, Default)]
pub struct Progress(Rc<Cell<(u64, u64)>>);

impl Progress {
    /// Bytes read so far, and the pack's size (0 until it is known).
    pub fn get(&self) -> (u64, u64) {
        self.0.get()
    }
}

/// The whole pack from `source`.
pub async fn read(source: &Source, progress: &Progress) -> Result<Vec<u8>, String> {
    let length = source.length().await?;
    progress.0.set((0, length));
    let mut bytes = Vec::with_capacity(length as usize);
    while (bytes.len() as u64) < length {
        let at = bytes.len() as u64;
        bytes.extend_from_slice(&source.range(at, READ.min(length - at)).await?);
        progress.0.set((bytes.len() as u64, length));
    }
    Ok(bytes)
}

fn rgb565(c: u16) -> [u32; 3] {
    let (r, g, b) = ((c >> 11) as u32, ((c >> 5) & 63) as u32, (c & 31) as u32);
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// One level of BC1 blocks as rows of RGBA.
pub fn bc1(blocks: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![255u8; w * h * 4];
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    for by in 0..bh {
        for bx in 0..bw {
            let b = &blocks[(by * bw + bx) * 8..][..8];
            let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
            let (a, c) = (rgb565(c0), rgb565(c1));
            let palette: [[u32; 3]; 4] = if c0 > c1 {
                [a, c, [0, 1, 2].map(|i| (2 * a[i] + c[i] + 1) / 3), [0, 1, 2].map(|i| (a[i] + 2 * c[i] + 1) / 3)]
            } else {
                [a, c, [0, 1, 2].map(|i| (a[i] + c[i]) / 2), [0; 3]]
            };
            let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x < w && y < h {
                        let p = palette[((bits >> ((py * 4 + px) * 2)) & 3) as usize];
                        out[(y * w + x) * 4..][..3].copy_from_slice(&[p[0] as u8, p[1] as u8, p[2] as u8]);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_of_two_colours() {
        // White and black, texels alternating by row: 0 0 0 0 / 1 1 1 1 / 2 2 2 2 / 3 3 3 3.
        let block = [0xff, 0xff, 0x00, 0x00, 0b0000_0000, 0b0101_0101, 0b1010_1010, 0b1111_1111];
        let px = bc1(&block, 4, 4);
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
        assert_eq!(&px[16..20], &[0, 0, 0, 255]);
        assert_eq!(&px[32..36], &[170, 170, 170, 255]);
        assert_eq!(&px[48..52], &[85, 85, 85, 255]);
    }
}
