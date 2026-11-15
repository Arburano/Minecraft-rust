use binary_greedy_meshing as bgm;
use noise::{NoiseFn, Perlin};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::time::Instant;

const CS: usize = 62;
const P: usize = CS + 2;
const P3: usize = P * P * P;
const SEA: i32 = 78;
// ids: 0 air, 1 stone, 2 dirt, 3 grass, 4 sand, 5 water(T), 6 leaves(T), 7 glass(T)
#[inline]
fn idx(x: usize, y: usize, z: usize) -> usize { (y * P + x) * P + z }

fn gen_chunk(cx: i32, cy: i32, cz: i32, h: &Perlin, c: &Perlin) -> Vec<u16> {
    let mut v = vec![0u16; P3];
    let ox = cx * CS as i32 - 1;
    let oy = cy * CS as i32 - 1;
    let oz = cz * CS as i32 - 1;
    // height map
    let mut hm = vec![0i32; P * P];
    for x in 0..P { for z in 0..P {
        let wx = (ox + x as i32) as f64; let wz = (oz + z as i32) as f64;
        let mut a = 1.0; let mut f = 1.0 / 128.0; let mut s = 0.0;
        for _ in 0..4 { s += a * h.get([wx * f, wz * f]); a *= 0.5; f *= 2.0; }
        hm[x * P + z] = (80.0 + s * 38.0) as i32;
    }}
    for y in 0..P { for x in 0..P { for z in 0..P {
        let wy = oy + y as i32;
        let height = hm[x * P + z];
        let wx = (ox + x as i32) as f64; let wz = (oz + z as i32) as f64;
        let mut b = 0u16;
        if wy <= height {
            b = if wy == height { if height <= SEA + 1 { 4 } else { 3 } }
                else if wy >= height - 3 { 2 } else { 1 };
            // caves (3D noise worms-ish)
            if wy < height - 2 {
                let n = c.get([wx / 24.0, wy as f64 / 16.0, wz / 24.0]);
                if n > 0.55 { b = 0; }
            }
        } else if wy <= SEA { b = 5; }
        v[idx(x, y, z)] = b;
    }}}
    // trees: leaves blobs + glass pillars deterministic pseudo-random
    let mut seed = (cx as u64).wrapping_mul(73856093) ^ (cz as u64).wrapping_mul(19349663) ^ (cy as u64) * 83492791 | 1;
    let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
    for _ in 0..14 {
        let x = 3 + (rnd() % (CS as u64 - 6)) as usize;
        let z = 3 + (rnd() % (CS as u64 - 6)) as usize;
        let gh = hm[x * P + z];
        if gh <= SEA + 1 { continue; }
        let gy = gh - oy;
        for dy in 1..=5 { let y = gy + dy; if y>=0 && (y as usize) < P && v[idx(x,y as usize,z)]==0 { v[idx(x,y as usize,z)] = 2; } }
        for dy in 4..=7 { for dx in -2i32..=2 { for dz in -2i32..=2 {
            if dx.abs()+dz.abs() > 3 { continue; }
            let (xx, yy, zz) = ((x as i32+dx), gy+dy, (z as i32+dz));
            if xx>=0 && yy>=0 && zz>=0 && (xx as usize)<P && (yy as usize)<P && (zz as usize)<P {
                let i = idx(xx as usize, yy as usize, zz as usize);
                if v[i]==0 { v[i]=6; }
            }
        }}}
    }
    v
}

fn naive_mesh(v: &[u16], trans: &BTreeSet<u16>, out: &mut Vec<u64>) {
    out.clear();
    let mut tt=[false;256]; for &t in trans { tt[t as usize]=true; }
    let is_t = |b: u16| tt[b as usize];
    for y in 1..=CS { for x in 1..=CS { for z in 1..=CS {
        let b = v[idx(x, y, z)];
        if b == 0 { continue; }
        let bt = is_t(b);
        let nb = [idx(x+1,y,z), idx(x-1,y,z), idx(x,y+1,z), idx(x,y-1,z), idx(x,y,z+1), idx(x,y,z-1)];
        for (f, &ni) in nb.iter().enumerate() {
            let n = v[ni];
            let vis = n == 0 || (is_t(n) && (!bt || n != b)) ;
            if vis { out.push(((x as u64)<<0) | ((y as u64)<<6) | ((z as u64)<<12) | ((f as u64)<<18) | ((b as u64)<<24)); }
        }
    }}}
}

fn masks(v: &[u16], trans: &BTreeSet<u16>) -> (Vec<u64>, Vec<u64>) {
    let mut o = vec![0u64; P * P]; let mut t = vec![0u64; P * P];
    for (i, &b) in v.iter().enumerate() {
        if b == 0 { continue; }
        let (r, q) = (i / P, i % P);
        if trans.contains(&b) { t[r] |= 1 << q } else { o[r] |= 1 << q }
    }
    (o, t)
}

fn stats(mut s: Vec<f64>) -> (f64, f64, f64) {
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (s[s.len()/2], s[(s.len() as f64 * 0.95) as usize], s.iter().sum::<f64>()/s.len() as f64)
}

fn main() {
    let hn = Perlin::new(1); let cn = Perlin::new(2);
    let trans: BTreeSet<u16> = [5u16, 6, 7].into_iter().collect();
    let t0 = Instant::now();
    let mut chunks = Vec::new();
    for cy in 1..=1 { for cx in 0..16 { for cz in 0..16 { chunks.push(gen_chunk(cx, cy, cz, &hn, &cn)); } } }
    println!("generated {} chunks (62^3 padded to 64^3), cy=1 layer (world y 61..123), in {:.1}s", chunks.len(), t0.elapsed().as_secs_f64());
    // composition
    let (mut air, mut solid, mut tr) = (0usize, 0usize, 0usize);
    for c in &chunks { for y in 1..=CS { for x in 1..=CS { for z in 1..=CS { let b = c[idx(x,y,z)]; if b==0 {air+=1} else if trans.contains(&b) {tr+=1} else {solid+=1} }}}}
    let tot = (chunks.len() * CS*CS*CS) as f64;
    println!("composition: air {:.1}%  opaque {:.1}%  transparent(water+leaves) {:.1}%", air as f64/tot*100.0, solid as f64/tot*100.0, tr as f64/tot*100.0);
    // also fraction of non-air that is transparent
    println!("transparent share of non-air: {:.1}%", tr as f64 / (tr+solid) as f64 * 100.0);

    let mut mesher = bgm::Mesher::<CS>::new();
    let ms: Vec<(Vec<u64>, Vec<u64>)> = chunks.iter().map(|c| masks(c, &trans)).collect();

    // single-thread timings, 5 passes
    let (mut tm, mut tf, mut tn, mut tmask) = (vec![], vec![], vec![], vec![]);
    let (mut qm, mut qf, mut qn) = (0usize, 0usize, 0usize);
    let mut nv = Vec::new();
    for pass in 0..6 {
        for (i, c) in chunks.iter().enumerate() {
            let t = Instant::now(); mesher.clear(); mesher.mesh(c, &trans); let d = t.elapsed().as_secs_f64()*1e6;
            if pass > 0 { tm.push(d); } if pass==1 { qm += mesher.quads.iter().map(|q| q.len()).sum::<usize>(); }
            let t = Instant::now(); mesher.clear(); mesher.fast_mesh(c, &ms[i].0, &ms[i].1); let d = t.elapsed().as_secs_f64()*1e6;
            if pass > 0 { tf.push(d); } if pass==1 { qf += mesher.quads.iter().map(|q| q.len()).sum::<usize>(); }
            let t = Instant::now(); naive_mesh(c, &trans, &mut nv); let d = t.elapsed().as_secs_f64()*1e6;
            if pass > 0 { tn.push(d); } if pass==1 { qn += nv.len(); }
            let t = Instant::now(); let m = masks(c, &trans); let d = t.elapsed().as_secs_f64()*1e6; std::hint::black_box(m);
            if pass > 0 { tmask.push(d); }
        }
    }
    let n = chunks.len() as f64;
    for (name, v, q) in [("bgm::mesh (BTreeSet)", tm, qm), ("bgm::fast_mesh (masks given)", tf, qf), ("naive per-face cull (no merge)", tn, qn)] {
        let (m, p, a) = stats(v);
        println!("{:32} median {:8.1} us  p95 {:8.1} us  mean {:8.1} us   quads/chunk {:.0}  ({:.0} KB at 8B/quad)", name, m, p, a, q as f64 / n, q as f64 / n * 8.0 / 1024.0);
    }
    let (m, p, _) = stats(tmask); println!("mask build from voxels (full rebuild, not needed per-mesh if maintained incrementally): median {:.1} us p95 {:.1} us", m, p);
    // empty / solid chunk reference
    let empty = vec![0u16; P3]; let solid_c = vec![1u16; P3];
    for (nm, c) in [("all-air", &empty), ("all-stone (padded)", &solid_c)] {
        let (o, t) = masks(c, &trans);
        let mut s = vec![]; for _ in 0..200 { let t0 = Instant::now(); mesher.clear(); mesher.mesh(c, &trans); s.push(t0.elapsed().as_secs_f64()*1e6); }
        let (m, p, _) = stats(s);
        let mut s2 = vec![]; for _ in 0..200 { let t0 = Instant::now(); mesher.clear(); mesher.fast_mesh(c, &o, &t); s2.push(t0.elapsed().as_secs_f64()*1e6); }
        let (m2, p2, _) = stats(s2);
        println!("{nm}: mesh median {m:.1} us p95 {p:.1} ; fast_mesh median {m2:.1} us p95 {p2:.1}; quads {}", mesher.quads.iter().map(|q| q.len()).sum::<usize>());
    }

    // rayon scaling over 256 chunks
    println!("\nrayon scaling (256 chunks x 20 rounds, mesher per worker; machine has {} logical CPUs)", std::thread::available_parallelism().unwrap());
    for mode in ["mesh", "fast_mesh"] {
        let mut base = 0.0;
        for th in [1usize, 2, 4, 8] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(th).build().unwrap();
            let mut best = 0.0f64;
            for _rep in 0..3 {
                let t = Instant::now();
                pool.install(|| {
                    for _ in 0..20 {
                        let q: usize = (0..chunks.len()).into_par_iter().map_init(|| bgm::Mesher::<CS>::new(), |m, i| {
                            m.clear();
                            if mode == "mesh" { m.mesh(&chunks[i], &trans); } else { m.fast_mesh(&chunks[i], &ms[i].0, &ms[i].1); }
                            m.quads.iter().map(|q| q.len()).sum::<usize>()
                        }).sum();
                        std::hint::black_box(q);
                    }
                });
                let cps = 256.0 * 20.0 / t.elapsed().as_secs_f64();
                best = best.max(cps);
            }
            if th == 1 { base = best; }
            println!("{mode:10} threads={th}: {best:8.0} chunks/s (best of 3)  speedup {:.2}x  -> {:.1} us/chunk wall", best / base, 1e6 / best);
        }
    }
}
