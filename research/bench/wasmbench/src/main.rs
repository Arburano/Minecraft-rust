use std::hint::black_box;
use std::time::Instant;
use wasmtime::*;

const CORE_WAT: &str = r#"
(module
  (import "host" "hook" (func $hook (param i32) (result i32)))
  (memory (export "memory") 2)
  (global $acc (mut i32) (i32.const 0))
  (func (export "empty"))
  (func (export "add") (param i32 i32) (result i32) (i32.add (local.get 0) (local.get 1)))
  ;; chunk lives at [0,8192) as u16; update buffer lives at 16384.. as (x,y,z,id) i32 x4
  (func (export "set_block") (param $x i32) (param $y i32) (param $z i32) (param $id i32)
    (i32.store16
      (i32.shl (i32.add (i32.add (local.get $x) (i32.shl (local.get $z) (i32.const 4))) (i32.shl (local.get $y) (i32.const 8))) (i32.const 1))
      (local.get $id)))
  (func (export "batch") (param $p i32) (param $n i32)
    (local $i i32) (local $q i32)
    (local.set $q (local.get $p))
    (block $done
      (loop $l
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store16
          (i32.shl (i32.add (i32.add (i32.load (local.get $q)) (i32.shl (i32.load offset=8 (local.get $q)) (i32.const 4))) (i32.shl (i32.load offset=4 (local.get $q)) (i32.const 8))) (i32.const 1))
          (i32.load offset=12 (local.get $q)))
        (local.set $q (i32.add (local.get $q) (i32.const 16)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l))))
  (func (export "call_host") (param $n i32) (result i32)
    (local $i i32) (local $a i32)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $a (i32.add (local.get $a) (call $hook (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $a))
  (func (export "spin") (param $n i32) (result i32)
    (local $a i32) (local $i i32)
    (local.set $a (i32.const 1))
    (loop $l
      (local.set $a (i32.add (i32.mul (local.get $a) (i32.const 1664525)) (i32.const 1013904223)))
      (local.set $a (i32.xor (local.get $a) (i32.shr_u (local.get $a) (i32.const 7))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (local.get $n))))
    (local.get $a))
  (func $fib (export "fib") (param $n i32) (result i32)
    (if (result i32) (i32.lt_u (local.get $n) (i32.const 2))
      (then (local.get $n))
      (else (i32.add (call $fib (i32.sub (local.get $n) (i32.const 1))) (call $fib (i32.sub (local.get $n) (i32.const 2)))))))
)"#;

const COMP_WAT: &str = r#"
(component
  (core module $m
    (memory (export "memory") 2)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 16384))
    (func (export "empty"))
    (func (export "add") (param i32 i32) (result i32) (i32.add (local.get 0) (local.get 1)))
    (func (export "set_block") (param $x i32) (param $y i32) (param $z i32) (param $id i32)
      (i32.store16
        (i32.shl (i32.add (i32.add (local.get $x) (i32.shl (local.get $z) (i32.const 4))) (i32.shl (local.get $y) (i32.const 8))) (i32.const 1))
        (local.get $id)))
    ;; list<u32> of (x,y,z,id) tuples flattened: (ptr,len_in_u32)
    (func (export "batch") (param $p i32) (param $len i32)
      (local $i i32) (local $q i32) (local $n i32)
      (local.set $q (local.get $p))
      (local.set $n (i32.shr_u (local.get $len) (i32.const 2)))
      (block $done (loop $l
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store16
          (i32.shl (i32.add (i32.add (i32.load (local.get $q)) (i32.shl (i32.load offset=8 (local.get $q)) (i32.const 4))) (i32.shl (i32.load offset=4 (local.get $q)) (i32.const 8))) (i32.const 1))
          (i32.load offset=12 (local.get $q)))
        (local.set $q (i32.add (local.get $q) (i32.const 16)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l)))))
  (core instance $i (instantiate $m))
  (func (export "empty") (canon lift (core func $i "empty")))
  (func (export "add") (param "a" u32) (param "b" u32) (result u32) (canon lift (core func $i "add")))
  (func (export "set-block") (param "x" u32) (param "y" u32) (param "z" u32) (param "id" u32) (canon lift (core func $i "set_block")))
  (func (export "batch") (param "b" (list u32)) (canon lift (core func $i "batch") (memory (core memory $i "memory")) (realloc (core func $i "realloc"))))
)"#;

fn med(mut v: Vec<f64>) -> (f64, f64) { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); (v[v.len() / 2], v[(v.len() as f64 * 0.95) as usize]) }

fn timeit<F: FnMut()>(reps: usize, ops_per_rep: usize, mut f: F) -> (f64, f64) {
    f(); // warm
    let mut s = vec![];
    for _ in 0..reps { let t = Instant::now(); f(); s.push(t.elapsed().as_nanos() as f64 / ops_per_rep as f64); }
    med(s)
}

#[derive(Clone, Copy)]
struct Cfg { name: &'static str, epoch: bool, fuel: bool }

fn make(cfg: Cfg) -> anyhow::Result<(Engine, Store<()>, Instance, Memory)> {
    let mut c = Config::new();
    c.epoch_interruption(cfg.epoch); c.consume_fuel(cfg.fuel);
    let engine = Engine::new(&c)?;
    let module = Module::new(&engine, CORE_WAT)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap("host", "hook", |x: i32| -> i32 { x.wrapping_add(1) })?;
    let mut store = Store::new(&engine, ());
    if cfg.epoch { store.set_epoch_deadline(u64::MAX / 4); }
    if cfg.fuel { store.set_fuel(u64::MAX / 2)?; }
    let inst = linker.instantiate(&mut store, &module)?;
    let mem = inst.get_memory(&mut store, "memory").unwrap();
    Ok((engine, store, inst, mem))
}

fn main() -> anyhow::Result<()> {
    println!("wasmtime crate version: {}", env!("CARGO_PKG_VERSION").to_string() + " (bench crate); see Cargo.lock for wasmtime");
    let cfgs = [Cfg { name: "baseline", epoch: false, fuel: false }, Cfg { name: "epoch", epoch: true, fuel: false }, Cfg { name: "fuel", epoch: false, fuel: true }, Cfg { name: "epoch+fuel", epoch: true, fuel: true }];

    println!("\n== Core module: per-call overhead (ns, median / p95 of 21 reps) ==");
    let order: Vec<Cfg> = cfgs.iter().copied().chain(cfgs.iter().rev().copied()).collect();
    for (round, cfg) in order.into_iter().enumerate() {
        if round % 4 == 0 { println!("-- round {} ({}) --", round / 4 + 1, if round == 0 { "forward order" } else { "reverse order, to expose warm-up/ordering effects" }); }
        let (_e, mut store, inst, mem) = make(cfg)?;
        let empty = inst.get_typed_func::<(), ()>(&mut store, "empty")?;
        let add = inst.get_typed_func::<(i32, i32), i32>(&mut store, "add")?;
        let set_block = inst.get_typed_func::<(i32, i32, i32, i32), ()>(&mut store, "set_block")?;
        let batch = inst.get_typed_func::<(i32, i32), ()>(&mut store, "batch")?;
        let (m, p) = timeit(21, 100_000, || { for _ in 0..100_000 { empty.call(&mut store, ()).unwrap(); } });
        println!("[{:10}] (a) empty call              : {m:8.1} / {p:8.1} ns", cfg.name);
        let (m, p) = timeit(21, 100_000, || { for i in 0..100_000 { black_box(add.call(&mut store, (i, 1)).unwrap()); } });
        println!("[{:10}]     add(i32,i32)->i32       : {m:8.1} / {p:8.1} ns", cfg.name);
        // (b1) 1000 updates individually
        let (m, p) = timeit(21, 200, || { for _ in 0..200 { for k in 0..1000i32 { set_block.call(&mut store, (k & 15, (k >> 4) & 15, (k >> 8) & 3, k & 255)).unwrap(); } } });
        println!("[{:10}] (b1) 1000 updates as 1000 calls: {:8.1} / {:8.1} ns per 1000-batch  = {:.1} ns/update", cfg.name, m * 200.0 / 200.0, p, m / 1000.0);
        // (b2) batched: host writes 16 KB into guest memory + one call
        let mut buf = vec![0u8; 16000];
        for k in 0..1000u32 { let w = [(k & 15), ((k >> 4) & 15), ((k >> 8) & 3), k & 255]; for (j, v) in w.iter().enumerate() { buf[(k as usize * 4 + j) * 4..][..4].copy_from_slice(&v.to_le_bytes()); } }
        let (m2, p2) = timeit(21, 200, || { for _ in 0..200 { mem.write(&mut store, 16384, &buf).unwrap(); batch.call(&mut store, (16384, 1000)).unwrap(); } });
        println!("[{:10}] (b2) 1000 updates, 1 batched call (memcpy 16KB + call): {m2:8.1} / {p2:8.1} ns per 1000-batch = {:.2} ns/update", cfg.name, m2 / 1000.0);
        // per-call fuel/epoch refill as a real host would do per hook invocation
        if cfg.fuel || cfg.epoch {
            let (m3, p3) = timeit(21, 100_000, || { for _ in 0..100_000 { if cfg.fuel { store.set_fuel(10_000).unwrap(); } if cfg.epoch { store.set_epoch_deadline(1); } empty.call(&mut store, ()).unwrap(); } });
            println!("[{:10}]     empty call incl. per-call set_fuel/set_epoch_deadline: {m3:8.1} / {p3:8.1} ns", cfg.name);
            if cfg.fuel { store.set_fuel(u64::MAX / 2)?; }
            if cfg.epoch { store.set_epoch_deadline(u64::MAX / 4); }
        }
    }

    println!("\n== Guest -> host import call (ns per call; guest loop calling host.hook) ==");
    for cfg in cfgs[..1].iter().copied() {
        let (_e, mut store, inst, _m) = make(cfg)?;
        let f = inst.get_typed_func::<i32, i32>(&mut store, "call_host")?;
        let (m, p) = timeit(21, 1_000_000, || { black_box(f.call(&mut store, 1_000_000).unwrap()); });
        println!("[{}] guest->host import call: {m:.1} / {p:.1} ns", cfg.name);
    }

    println!("\n== Compute-heavy guest: epoch / fuel overhead (relative to baseline, median of 7 reps) ==");
    for (label, fname, arg) in [("tight loop spin(300M iters)", "spin", 300_000_000i32), ("call-heavy fib(35)", "fib", 35)] {
        let mut base = 0.0;
        for cfg in cfgs {
            let (_e, mut store, inst, _m) = make(cfg)?;
            let f = inst.get_typed_func::<i32, i32>(&mut store, fname)?;
            let mut s = vec![];
            f.call(&mut store, if fname == "fib" { 25 } else { 1000 })?;
            for _ in 0..7 { let t = Instant::now(); black_box(f.call(&mut store, arg)?); s.push(t.elapsed().as_secs_f64() * 1000.0); }
            let (m, p) = med(s);
            if cfg.name == "baseline" { base = m; }
            println!("{label:30} [{:10}] {m:9.1} ms (p95 {p:.1})  {:+.1}% vs baseline", cfg.name, (m / base - 1.0) * 100.0);
        }
    }

    println!("\n== Epoch interrupt actually works? (ticker thread + deadline trap) ==");
    {
        let (engine, mut store, inst, _m) = make(cfgs[1])?;
        store.set_epoch_deadline(1);
        let e2 = engine.clone();
        std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(20)); e2.increment_epoch(); });
        let f = inst.get_typed_func::<i32, i32>(&mut store, "spin")?;
        let t = Instant::now();
        let r = f.call(&mut store, i32::MAX);
        println!("spin(i32::MAX) interrupted after {:.1} ms (expected ~20): {}", t.elapsed().as_secs_f64() * 1000.0, if r.is_err() { "trapped OK" } else { "NOT trapped" });
    }
    println!("\n== Instantiation cost (InstancePre), baseline config ==");
    {
        let mut c = Config::new(); c.epoch_interruption(true);
        let engine = Engine::new(&c)?;
        let t = Instant::now(); let module = Module::new(&engine, CORE_WAT)?; println!("compile tiny module: {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        let mut linker = Linker::new(&engine); linker.func_wrap("host", "hook", |x: i32| -> i32 { x + 1 })?;
        let pre = linker.instantiate_pre(&module)?;
        let (m, p) = timeit(21, 1000, || { for _ in 0..1000 { let mut st = Store::new(&engine, ()); black_box(pre.instantiate(&mut st).unwrap()); } });
        println!("new Store + instantiate(pre): {:.2} us median / {:.2} us p95", m / 1000.0, p / 1000.0);
    }

    println!("\n== Component Model (hand-written component in WAT, wasmtime::component) ==");
    match component_bench() { Ok(()) => {}, Err(e) => println!("COMPONENT BENCH FAILED: {e:#}") }
    Ok(())
}

fn component_bench() -> anyhow::Result<()> {
    use wasmtime::component::{Component, Linker};
    for (name, epoch, fuel) in [("baseline", false, false), ("epoch", true, false), ("epoch+fuel", true, true)] {
        let mut c = Config::new(); c.epoch_interruption(epoch); c.consume_fuel(fuel);
        let engine = Engine::new(&c)?;
        let comp = Component::new(&engine, COMP_WAT)?;
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        if epoch { store.set_epoch_deadline(u64::MAX / 4); }
        if fuel { store.set_fuel(u64::MAX / 2)?; }
        let inst = linker.instantiate(&mut store, &comp)?;
        let empty = inst.get_typed_func::<(), ()>(&mut store, "empty")?;
        let add = inst.get_typed_func::<(u32, u32), (u32,)>(&mut store, "add")?;
        let set_block = inst.get_typed_func::<(u32, u32, u32, u32), ()>(&mut store, "set-block")?;
        let batch = inst.get_typed_func::<(Vec<u32>,), ()>(&mut store, "batch")?;
        let (m, p) = timeit(21, 100_000, || { for _ in 0..100_000 { empty.call(&mut store, ()).unwrap(); } });
        println!("[comp {name:10}] (a) empty call: {m:8.1} / {p:8.1} ns");
        let (m, p) = timeit(21, 100_000, || { for i in 0..100_000u32 { black_box(add.call(&mut store, (i, 1)).unwrap()); } });
        println!("[comp {name:10}]     add(u32,u32)->u32: {m:8.1} / {p:8.1} ns");
        let (m, p) = timeit(21, 200, || { for _ in 0..200 { for k in 0..1000u32 { set_block.call(&mut store, (k & 15, (k >> 4) & 15, (k >> 8) & 3, k & 255)).unwrap(); } } });
        println!("[comp {name:10}] (b1) 1000 updates as 1000 calls: {m:8.1} / {p:8.1} ns per batch = {:.1} ns/update", m / 1000.0);
        let data: Vec<u32> = (0..1000u32).flat_map(|k| [k & 15, (k >> 4) & 15, (k >> 8) & 3, k & 255]).collect();
        let (m, p) = timeit(21, 200, || { for _ in 0..200 { batch.call(&mut store, (data.clone(),)).unwrap(); } });
        println!("[comp {name:10}] (b2) 1000 updates, 1 call with list<u32> (clone+lower 16KB): {m:8.1} / {p:8.1} ns per batch = {:.2} ns/update", m / 1000.0);
    }
    Ok(())
}
