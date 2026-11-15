use std::hint::black_box;
use std::time::Instant;

struct R(u64);
impl R { fn n(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }

struct Grid { n: usize, opaque: Vec<bool>, light: Vec<u8>, queue: Vec<u32> }

impl Grid {
    fn new(n: usize) -> Self { Self { n, opaque: vec![false; n * n * n], light: vec![0; n * n * n], queue: Vec::with_capacity(n * n * n) } }
    #[inline] fn i(&self, x: usize, y: usize, z: usize) -> usize { (y * self.n + z) * self.n + x }
    /// BFS flood-fill over queue; returns number of pops
    fn flood(&mut self) -> usize {
        let n = self.n; let n2 = n * n;
        let mut head = 0;
        while head < self.queue.len() {
            let idx = self.queue[head] as usize; head += 1;
            let l = self.light[idx];
            if l <= 1 { continue; }
            let x = idx % n; let z = (idx / n) % n; let y = idx / n2;
            let nl = l - 1;
            macro_rules! visit { ($cond:expr, $ni:expr) => { if $cond { let ni = $ni; if !self.opaque[ni] && self.light[ni] < nl { self.light[ni] = nl; self.queue.push(ni as u32); } } } }
            visit!(x + 1 < n, idx + 1);
            visit!(x > 0, idx.wrapping_sub(1));
            visit!(z + 1 < n, idx + n);
            visit!(z > 0, idx.wrapping_sub(n));
            visit!(y + 1 < n, idx + n2);
            visit!(y > 0, idx.wrapping_sub(n2));
        }
        head
    }
}

fn stats(mut s: Vec<f64>) -> (f64, f64) { s.sort_by(|a, b| a.partial_cmp(b).unwrap()); (s[s.len() / 2], s[(s.len() as f64 * 0.95) as usize]) }

fn fill_blocks(g: &mut Grid, r: &mut R, density_pct: u64) {
    // blobby opaque: random seeds grown a bit
    let n = g.n; let total = n * n * n;
    for o in g.opaque.iter_mut() { *o = false; }
    for _ in 0..(total * density_pct as usize / 100 / 6) {
        let (x, y, z) = ((r.n() % n as u64) as usize, (r.n() % n as u64) as usize, (r.n() % n as u64) as usize);
        for (dx, dy, dz) in [(0i32,0i32,0i32),(1,0,0),(0,1,0),(0,0,1),(-1,0,0),(0,-1,0)] {
            let (xx, yy, zz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
            if xx >= 0 && yy >= 0 && zz >= 0 && (xx as usize) < n && (yy as usize) < n && (zz as usize) < n { let i = g.i(xx as usize, yy as usize, zz as usize); g.opaque[i] = true; }
        }
    }
}

fn block_light(g: &mut Grid, sources: &[usize], level: u8) -> usize {
    g.light.fill(0); g.queue.clear();
    for &s in sources { g.light[s] = level; g.queue.push(s as u32); }
    g.flood()
}

fn bench<F: FnMut() -> usize>(label: &str, reps: usize, mut f: F, cells: usize) {
    f();
    let mut s = vec![]; let mut pops = 0;
    for _ in 0..reps { let t = Instant::now(); pops = black_box(f()); s.push(t.elapsed().as_nanos() as f64 / 1000.0); }
    let (m, p) = stats(s);
    println!("{label:58} median {m:9.1} us  p95 {p:9.1} us   queue pops {pops:7} / {cells} cells  ({:.1} ns/pop)", m * 1000.0 / pops.max(1) as f64);
}

fn main() {
    let mut r = R(0xdeadbeefcafe);
    for n in [16usize, 48] {
        let cells = n * n * n;
        let secs = (n / 16).pow(3);
        println!("\n=== grid {n}^3 ({} section{}) ===", secs, if secs > 1 { "s: 3x3x3 neighbourhood" } else { "" });
        // open air, 1 torch in centre (worst-case spread)
        let mut g = Grid::new(n);
        let c = g.i(n / 2, n / 2, n / 2);
        bench("open air, 1 torch (lvl 14)", 301, || block_light(&mut g, &[c], 14), cells);
        // open air, 8 torches per section
        let mut src = vec![]; for _ in 0..8 * secs { src.push(g.i((r.n() % n as u64) as usize, (r.n() % n as u64) as usize, (r.n() % n as u64) as usize)); }
        bench("open air, 8 torches/section", 201, || block_light(&mut g, &src, 14), cells);
        // cave-like: 30% opaque
        for d in [30u64, 60] {
            fill_blocks(&mut g, &mut r, d);
            let mut src = vec![]; while src.len() < 8 * secs { let i = g.i((r.n() % n as u64) as usize, (r.n() % n as u64) as usize, (r.n() % n as u64) as usize); if !g.opaque[i] { src.push(i); } }
            let solid = g.opaque.iter().filter(|b| **b).count() as f64 / cells as f64 * 100.0;
            bench(&format!("{:.0}% opaque, 8 torches/section", solid), 301, || block_light(&mut g, &src, 14), cells);
        }
        // skylight: terrain heightmap, columns from top
        let mut hm = vec![0usize; n * n]; for x in 0..n { for z in 0..n { hm[x * n + z] = n / 2 + ((x * 3 + z * 5) % 7) + ((x / 4 + z / 3) % 3); } }
        for y in 0..n { for z in 0..n { for x in 0..n { let i = g.i(x, y, z); let roof = ((x / 5 + z / 5) % 3 == 0) && (y == n - 3 || y == n - 4); g.opaque[i] = y < hm[x * n + z] || roof; } } }
        let sky = |g: &mut Grid| -> usize {
            g.light.fill(0); g.queue.clear();
            let n = g.n;
            for x in 0..n { for z in 0..n { let mut y = n; while y > 0 { y -= 1; let i = g.i(x, y, z); if g.opaque[i] { break; } g.light[i] = 15; } } }
            // seed: lit cells adjacent to a darker transparent cell (sides)
            for y in 0..n { for z in 0..n { for x in 0..n {
                let i = g.i(x, y, z);
                if g.light[i] != 15 { continue; }
                let mut edge = false;
                if x + 1 < n && !g.opaque[i + 1] && g.light[i + 1] < 15 { edge = true; }
                if x > 0 && !g.opaque[i - 1] && g.light[i - 1] < 15 { edge = true; }
                if z + 1 < n && !g.opaque[i + n] && g.light[i + n] < 15 { edge = true; }
                if z > 0 && !g.opaque[i - n] && g.light[i - n] < 15 { edge = true; }
                if edge { g.queue.push(i as u32); }
            }}}
            g.flood()
        };
        bench("skylight: heightmap + roof patches (col pass + side BFS)", 301, || sky(&mut g), cells);
    }
}
