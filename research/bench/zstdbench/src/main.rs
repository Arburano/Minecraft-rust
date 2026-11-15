use noise::{NoiseFn, Perlin};
use std::hint::black_box;
use std::time::Instant;

const H: usize = 384; // 24 sections
fn gen_column(cx: i32, cz: i32, h: &Perlin, c: &Perlin, o: &Perlin) -> Vec<u16> {
    // layout: section-major, each section 16^3 with idx (y*16+z)*16+x ; returns H*256 voxels
    let mut v = vec![0u16; H * 256];
    for z in 0..16usize { for x in 0..16usize {
        let wx = (cx * 16 + x as i32) as f64; let wz = (cz * 16 + z as i32) as f64;
        let mut a = 1.0; let mut f = 1.0 / 128.0; let mut s = 0.0;
        for _ in 0..4 { s += a * h.get([wx * f, wz * f]); a *= 0.5; f *= 2.0; }
        let surf = (140.0 + s * 45.0) as usize; // world y offset +64
        for y in 0..H {
            let b = if y > surf { if y <= 128 { 9 } else { 0 } }
                else if y == surf { if surf <= 129 { 4 } else { 3 } }
                else if y + 4 >= surf { 2 }
                else if y < 4 { 7 }
                else {
                    let cave = y + 2 < surf && c.get([wx / 24.0, y as f64 / 14.0, wz / 24.0]) > 0.55;
                    if cave { 0 } else {
                        let on = o.get([wx / 5.0, y as f64 / 5.0, wz / 5.0]);
                        if on > 0.80 { 14 } else if on < -0.85 { 15 } else if on > 0.7 && y < 60 { 56 } else { 1 }
                    }
                };
            v[(y / 16) * 4096 + ((y % 16) * 16 + z) * 16 + x] = b;
        }
    }}
    v
}

fn pack_section(a: &[u16], out: &mut Vec<u8>) {
    let mut pal: Vec<u16> = vec![]; for &x in a { if !pal.contains(&x) { pal.push(x); } }
    if pal.len() == 1 { out.push(0); out.extend_from_slice(&pal[0].to_le_bytes()); return; }
    let bits = ((usize::BITS - (pal.len() - 1).leading_zeros()).max(1) as usize).max(4).min(16);
    out.push(bits as u8); out.push(pal.len() as u8); out.push((pal.len() >> 8) as u8);
    for p in &pal { out.extend_from_slice(&p.to_le_bytes()); }
    let per = 64 / bits; let words = (4096 + per - 1) / per;
    let mut data = vec![0u64; words];
    for (i, x) in a.iter().enumerate() { let id = pal.iter().position(|p| p == x).unwrap() as u64; data[i / per] |= id << ((i % per) * bits); }
    for w in data { out.extend_from_slice(&w.to_le_bytes()); }
}

fn bench_codec(name: &str, samples: &[Vec<u8>], test: &[Vec<u8>], dict: Option<&[u8]>) {
    for level in [1i32, 3, 9] {
        let mut comp = match dict { Some(d) => zstd::bulk::Compressor::with_dictionary(level, d).unwrap(), None => zstd::bulk::Compressor::new(level).unwrap() };
        let mut dec = match dict { Some(d) => zstd::bulk::Decompressor::with_dictionary(d).unwrap(), None => zstd::bulk::Decompressor::new().unwrap() };
        let _ = samples;
        let raw: usize = test.iter().map(|t| t.len()).sum();
        let mut outs = Vec::new(); let mut csize = 0;
        let mut ct = vec![];
        for rep in 0..5 { let t = Instant::now(); outs.clear(); for s in test { outs.push(comp.compress(s).unwrap()); } ct.push(t.elapsed().as_secs_f64()); if rep == 0 { csize = outs.iter().map(|o| o.len()).sum(); } }
        let mut dt = vec![];
        for _ in 0..5 { let t = Instant::now(); for (o, s) in outs.iter().zip(test) { black_box(dec.decompress(o, s.len()).unwrap()); } dt.push(t.elapsed().as_secs_f64()); }
        ct.sort_by(|a, b| a.partial_cmp(b).unwrap()); dt.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = test.len() as f64;
        println!("{name:34} level {level}: ratio {:6.2}x  ({:.0} B/col avg raw {:.0})  compress {:7.0} MB/s ({:6.1} us/col)  decompress {:7.0} MB/s ({:6.1} us/col)",
            raw as f64 / csize as f64, csize as f64 / n, raw as f64 / n, raw as f64 / ct[2] / 1e6, ct[2] / n * 1e6, raw as f64 / dt[2] / 1e6, dt[2] / n * 1e6);
    }
}

fn main() {
    let (h, c, o) = (Perlin::new(1), Perlin::new(2), Perlin::new(3));
    let t = Instant::now();
    let mut raw = vec![]; let mut packed = vec![];
    for cx in 0..20 { for cz in 0..20 {
        let col = gen_column(cx, cz, &h, &c, &o);
        let mut r = Vec::with_capacity(col.len() * 2); for v in &col { r.extend_from_slice(&v.to_le_bytes()); }
        let mut p = Vec::new(); for s in 0..24 { pack_section(&col[s * 4096..(s + 1) * 4096], &mut p); }
        raw.push(r); packed.push(p);
    }}
    println!("generated 400 columns (16x16x384) in {:.1}s; raw u16 column = {} B; palette-packed avg = {:.0} B", t.elapsed().as_secs_f64(), raw[0].len(), packed.iter().map(|p| p.len()).sum::<usize>() as f64 / 400.0);
    // split: train on even columns (200), test on 100 held-out odd columns
    for (label, set) in [("raw u16 column", &raw), ("palette-packed column", &packed)] {
        let train: Vec<Vec<u8>> = set.iter().step_by(2).take(200).cloned().collect();
        let test: Vec<Vec<u8>> = set.iter().skip(1).step_by(4).take(100).cloned().collect();
        println!("\n--- {label}: train 200 cols, test 100 held-out cols ---");
        bench_codec(&format!("{label}, no dict"), &train, &test, None);
        for dsize in [16 * 1024usize, 110 * 1024] {
            let t = Instant::now();
            match zstd::dict::from_samples(&train, dsize) {
                Ok(d) => { println!("(trained dict {} KB -> {} B in {:.1}s)", dsize / 1024, d.len(), t.elapsed().as_secs_f64()); bench_codec(&format!("{label}, dict {}KB", dsize / 1024), &train, &test, Some(&d)); }
                Err(e) => println!("dict training {} KB FAILED: {e}", dsize / 1024),
            }
        }
    }
    // also small payloads: dictionary matters most for single sections
    let sec: Vec<Vec<u8>> = (0..400).flat_map(|i| { let mut v = vec![]; let col = &raw[i]; for s in 6..14 { v.push(col[s * 8192..(s + 1) * 8192].to_vec()); } v }).collect();
    let train: Vec<Vec<u8>> = sec.iter().step_by(2).cloned().collect(); let test: Vec<Vec<u8>> = sec.iter().skip(1).step_by(10).cloned().collect();
    println!("\n--- single raw 16^3 section (8 KB), {} train / {} test ---", train.len(), test.len());
    bench_codec("section raw, no dict", &train, &test, None);
    if let Ok(d) = zstd::dict::from_samples(&train, 16 * 1024) { bench_codec("section raw, dict 16KB", &train, &test, Some(&d)); }
}
