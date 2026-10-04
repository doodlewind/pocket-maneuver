//! The atlas for the device: a mip chain filtered inside each strip, then BC1.

use maneuver_pack::TexHeader;

/// Level `level` of an RGBA image whose rows belong to strips delimited by
/// `edges`. Each output texel averages the level-0 texels under it, taking
/// only rows of the strip that holds the block's centre, so strips never mix.
fn mip(rgba: &[u8], w: usize, h: usize, edges: &[usize], level: u32) -> (Vec<u8>, usize, usize) {
    let f = 1usize << level;
    let (mw, mh) = ((w / f).max(1), (h / f).max(1));
    let strip_of = |row: usize| edges.windows(2).position(|e| row >= e[0] && row < e[1]).unwrap_or(0);
    let mut out = vec![0u8; mw * mh * 4];
    for y in 0..mh {
        let (y0, y1) = (y * f, ((y + 1) * f).min(h));
        let s = strip_of((y0 + y1) / 2);
        let (r0, r1) = (y0.max(edges[s]), y1.min(edges[s + 1]));
        for x in 0..mw {
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for yy in r0..r1 {
                for xx in x * f..((x + 1) * f).min(w) {
                    let o = (yy * w + xx) * 4;
                    for c in 0..4 {
                        sum[c] += rgba[o + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = (y * mw + x) * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] + n / 2) / n.max(1)) as u8;
            }
        }
    }
    (out, mw, mh)
}

/// `TexHeader` followed by BC1 levels, largest first.
pub fn atlas(rgba: &[u8], w: usize, h: usize, edges: &[usize], max_mips: u32) -> (Vec<u8>, u32) {
    let mut levels = 0;
    let mut data = Vec::new();
    for level in 0..max_mips {
        let (px, mw, mh) = mip(rgba, w, h, edges, level);
        if mw < 4 || mh < 4 {
            break;
        }
        let fmt = texpresso::Format::Bc1;
        let mut out = vec![0u8; fmt.compressed_size(mw, mh)];
        let params = texpresso::Params { algorithm: texpresso::Algorithm::ClusterFit, ..Default::default() };
        fmt.compress(&px, mw, mh, params, &mut out);
        data.extend_from_slice(&out);
        levels += 1;
    }
    let head = TexHeader { width: w as u32, height: h as u32, mips: levels, format: 1 };
    let mut bytes = maneuver_pack::bytes_of(&head).to_vec();
    bytes.extend_from_slice(&data);
    (bytes, levels)
}
