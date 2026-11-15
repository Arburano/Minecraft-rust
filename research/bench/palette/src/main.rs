use std::hint::black_box;
use std::time::Instant;

const N: usize = 4096;

/// Palette-compressed 16^3 section. Entries never straddle u64 words (Minecraft >=1.16 style).
#[derive(Clone)]
enum Section {
    Single(u16),
    Pal { bits: u8, palette: Vec<u16>, data: Vec<u64> },
    Direct(Vec<u64>), // 12-bit raw ids (covers up to 4096 distinct), 5 entries per word
}

fn words_for(bits: u8) -> usize { let per = 64 / bits as usize; (N + per - 1) / per }

impl Section {
    fn heap_bytes(&self) -> usize {
        // palette stored as Vec<u16>: count capacity-exact len*2
        std::mem::size_of::<Section>() + match self {
            Section::Single(_) => 0,
            Section::Pal { palette, data, .. } => palette.len() * 2 + data.len() * 8,
            Section::Direct(d) => d.len() * 8,
        }
    }
    #[inline]
    fn get_raw(data: &[u64], bits: u8, i: usize) -> u64 {
        let per = 64 / bits as usize;
        let w = data[i / per];
        (w >> ((i % per) * bits as usize)) & ((1u64 << bits) - 1)
    }
    #[inline]
    fn set_raw(data: &mut [u64], bits: u8, i: usize, v: u64) {
        let per = 64 / bits as usize;
        let sh = (i % per) * bits as usize;
        let m = ((1u64 << bits) - 1) << sh;
        let w = &mut data[i / per];
        *w = (*w & !m) | (v << sh);
    }
    #[inline]
    fn get(&self, i: usize) -> u16 {
        match self {
            Section::Single(v) => *v,
            Section::Pal { bits, palette, data } => palette[Self::get_raw(data, *bits, i) as usize],
            Section::Direct(d) => Self::get_raw(d, 12, i) as u16,
        }
    }
    fn repack(&mut self, new_bits: u8, new_palette: Option<Vec<u16>>) {
        // read everything with current repr, write with new
        let all: Vec<u16> = (0..N).map(|i| self.get(i)).collect();
        match (new_bits, new_palette) {
            (12, _) => {
                let mut d = vec![0u64; words_for(12)];
                for (i, v) in all.iter().enumerate() { Self::set_raw(&mut d, 12, i, *v as u64); }
                *self = Section::Direct(d);
            }
            (b, Some(pal)) => {
                let mut d = vec![0u64; words_for(b)];
                for (i, v) in all.iter().enumerate() {
                    let idx = pal.iter().position(|p| p == v).unwrap();
                    Self::set_raw(&mut d, b, i, idx as u64);
                }
                *self = Section::Pal { bits: b, palette: pal, data: d };
            }
            _ => unreachable!(),
        }
    }
    fn set(&mut self, i: usize, v: u16) {
        match self {
            Section::Single(s) => {
                if *s == v { return; }
                let pal = vec![*s, v];
                self.repack(1, Some(pal));
                self.set(i, v);
            }
            Section::Pal { bits, palette, data } => {
                let idx = match palette.iter().position(|&p| p == v) {
                    Some(x) => x,
                    None => {
                        if palette.len() < (1usize << *bits) {
                            palette.push(v); palette.len() - 1
                        } else {
                            // grow; bits 1..=8 paletted, above 8 -> direct
                            let nb = *bits + 1;
                            if nb <= 8 {
                                let mut np = palette.clone(); np.push(v);
                                self.repack(nb, Some(np));
                            } else {
                                self.repack(12, None);
                            }
                            self.set(i, v);
                            return;
                        }
                    }
                };
                Self::set_raw(data, *bits, i, idx as u64);
            }
            Section::Direct(d) => Self::set_raw(d, 12, i, v as u64),
        }
    }
    fn from_slice(a: &[u16]) -> Self {
        // build optimally: distinct values -> bits
        let mut pal: Vec<u16> = vec![];
        for &v in a { if !pal.contains(&v) { pal.push(v); } }
        if pal.len() == 1 { return Section::Single(pal[0]); }
        if pal.len() > 256 {
            let mut d = vec![0u64; words_for(12)];
            for (i, v) in a.iter().enumerate() { Self::set_raw(&mut d, 12, i, *v as u64); }
            return Section::Direct(d);
        }
        let bits = (usize::BITS - (pal.len() - 1).leading_zeros()).max(1) as u8;
        let mut d = vec![0u64; words_for(bits)];
        for (i, v) in a.iter().enumerate() { Self::set_raw(&mut d, bits, i, pal.iter().position(|p| p == v).unwrap() as u64); }
        Section::Pal { bits, palette: pal, data: d }
    }
    fn bits(&self) -> u8 { match self { Section::Single(_) => 0, Section::Pal{bits,..} => *bits, Section::Direct(_) => 12 } }
}

// deterministic xorshift
struct R(u64);
impl R { fn n(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }

fn idx(x: usize, y: usize, z: usize) -> usize { (y * 16 + z) * 16 + x }

fn mixed_terrain(kind: u32) -> Vec<u16> {
    // kind 0: surface section (air/grass/dirt/stone + few ores + water), kind 1: underground w/ ores and caves
    let mut r = R(0x1234_5678_9abc_def1 + kind as u64);
    let mut a = vec![0u16; N];
    for x in 0..16 { for z in 0..16 {
        let h = 6 + (r.n() % 6) as usize; // surface height inside section
        for y in 0..16 {
            let b = if kind == 0 {
                if y > h { if y < 8 { 9 } else { 0 } } else if y == h { 3 } else if y + 3 >= h { 2 } else { 1 }
            } else { 1 };
            a[idx(x, y, z)] = b;
        }
    }}
    if kind == 0 { for _ in 0..6 { let p = (r.n() % N as u64) as usize; if a[p] == 0 { a[p] = 6; } } } // leaves
    // ores + caves
    let ores = [14u16, 15, 16, 21, 56, 73, 129];
    for _ in 0..(if kind == 0 { 10 } else { 40 }) { let p = (r.n() % N as u64) as usize; if a[p] == 1 { a[p] = ores[(r.n() % ores.len() as u64) as usize]; } }
    if kind == 1 { for _ in 0..600 { let p = (r.n() % N as u64) as usize; a[p] = 0; } for _ in 0..40 { let p = (r.n() % N as u64) as usize; a[p] = 8; } }
    a
}

fn bench_ns<F: FnMut() -> u64>(reps: usize, ops: usize, mut f: F) -> (f64, f64) {
    let mut s = vec![];
    for _ in 0..reps { let t = Instant::now(); black_box(f()); s.push(t.elapsed().as_nanos() as f64 / ops as f64); }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (s[s.len() / 2], s[(s.len() as f64 * 0.95) as usize])
}

fn main() {
    let mut r = R(42);
    // --- sizes
    let air = vec![0u16; N];
    let stone = vec![1u16; N];
    let mixed = mixed_terrain(0);
    let under = mixed_terrain(1);
    let worst: Vec<u16> = (0..N as u16).collect();
    let mut w256 = vec![0u16; N]; for i in 0..N { w256[i] = (i % 256) as u16; }
    println!("plain [u16;4096] = {} B", N * 2);
    let cases: Vec<(&str, Vec<u16>)> = vec![("air", air), ("stone", stone), ("surface mixed", mixed.clone()), ("underground+caves+ores", under.clone()), ("256 distinct (8-bit)", w256), ("worst: 4096 distinct", worst)];
    let mut sizes = vec![];
    for (n, a) in &cases {
        let s = Section::from_slice(a);
        let distinct = { let mut v = a.clone(); v.sort(); v.dedup(); v.len() };
        let b = s.heap_bytes();
        // verify roundtrip
        for i in 0..N { assert_eq!(s.get(i), a[i]); }
        println!("{:26} distinct={:5} bits={:2} bytes={:5}  ({:.1}% of plain; incl. 'size_of::<Section>()'={} B enum overhead, excl. Vec heap headers)", n, distinct, s.bits(), b, b as f64 / 8192.0 * 100.0, std::mem::size_of::<Section>());
        sizes.push((n.to_string(), b));
    }
    // synthetic bit-width sizes
    for bits in 1..=8u8 {
        let mut a = vec![0u16; N]; let np = 1usize << bits; for i in 0..N { a[i] = (r.n() % np as u64) as u16; }
        let s = Section::from_slice(&a);
        println!("random with {np:3} palette entries -> bits {} bytes {}", s.bits(), s.heap_bytes());
    }

    // --- get/set speed
    let reps = 51;
    let order: Vec<usize> = (0..N).map(|_| (r.n() % N as u64) as usize).collect();
    println!("\nget/set ns per op (median / p95 over {reps} runs of 4096 ops; random index order; black_box'd)");
    // plain
    let mut plain = mixed.clone();
    let (m, p) = bench_ns(reps, N, || { let mut s = 0u64; for &i in &order { s += black_box(&plain)[i] as u64; } s });
    println!("plain u16 array   get seq-random : {m:6.2} / {p:6.2}");
    let (m, p) = bench_ns(reps, N, || { let mut s = 0u64; for i in 0..N { s += black_box(&plain)[i] as u64; } s });
    println!("plain u16 array   get sequential : {m:6.2} / {p:6.2}");
    let (m, p) = bench_ns(reps, N, || { for (k, &i) in order.iter().enumerate() { black_box(&mut plain)[i] = (k & 7) as u16; } 0 });
    println!("plain u16 array   set random     : {m:6.2} / {p:6.2}");

    for (name, base) in [("single(stone)", vec![1u16; N]), ("surface mixed", mixed.clone()), ("underground", under.clone()), ("256 distinct", cases[4].1.clone()), ("direct 12-bit", cases[5].1.clone())] {
        let mut s = Section::from_slice(&base);
        let (m, p) = bench_ns(reps, N, || { let mut a = 0u64; for &i in &order { a += black_box(&s).get(i) as u64; } a });
        println!("{:14} (bits {:2}) get random : {m:6.2} / {p:6.2}", name, s.bits());
        let (m, p) = bench_ns(reps, N, || { let mut a = 0u64; for i in 0..N { a += black_box(&s).get(i) as u64; } a });
        println!("{:14} (bits {:2}) get seq    : {m:6.2} / {p:6.2}", name, s.bits());
        // set: write values already in the palette (no growth)
        let vals: Vec<u16> = (0..N).map(|k| base[(k * 7) % N]).collect();
        let (m, p) = bench_ns(reps, N, || { for (k, &i) in order.iter().enumerate() { black_box(&mut s).set(i, vals[k]); } 0 });
        println!("{:14} (bits {:2}) set random existing-palette-entry: {m:6.2} / {p:6.2}", name, s.bits());
    }
    // growth cost: start Single, set distinct values to force 0->1->2->..->8->direct
    let mut s = vec![];
    for _ in 0..21 { let t = Instant::now(); let mut sec = Section::Single(0); for k in 0..300u16 { sec.set((k as usize * 13) % N, k + 1); } black_box(&sec); s.push(t.elapsed().as_micros() as f64); }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("\ngrowth: Single -> 300 distinct values inserted one by one (all repacks 1..8 bit and direct): median {:.1} us total (~{:.0} ns/set amortised)", s[10], s[10] * 1000.0 / 300.0);

    // --- client RAM estimate: 25x25 columns x 24 sections
    let sections = 25 * 25 * 24usize;
    let b_air = sizes[0].1; let b_stone = sizes[1].1; let b_surf = sizes[2].1; let b_under = sizes[3].1;
    println!("\nclient RAM estimate: 25x25 columns x 24 sections = {sections} sections");
    println!("plain u16 everywhere: {:.1} MiB", sections as f64 * 8192.0 / 1048576.0);
    // Assumed mix for a 384-block-tall world (24 sections): 8 above surface = air, 1.5 surface, 14.5 underground (some stone-only)
    let mix = [("air", 8.0, b_air), ("surface mixed", 1.5, b_surf), ("underground", 12.0, b_under), ("stone-only", 2.5, b_stone)];
    let tot: f64 = mix.iter().map(|(_, n, b)| n * *b as f64).sum::<f64>() * 625.0;
    println!("assumed per-column mix (8 air, 1.5 surface, 12 underground, 2.5 pure stone): {:.2} MiB", tot / 1048576.0);
    let worst_all = sections as f64 * sizes[5].1 as f64;
    println!("worst case (all sections 4096-distinct 12-bit direct): {:.1} MiB", worst_all / 1048576.0);
    let all8 = sections as f64 * sizes[4].1 as f64; println!("all sections at 8 bit palette: {:.1} MiB", all8 / 1048576.0);
}
