const fs = require('fs');
const dir = 'C:/Users/aboba/Documents/Qoder/2026-11-18/89794042/Minecraft-rust/docs/research/bare-metal-2026-11-22-t1-04';
const mult = { ns: 1, 'µs': 1e3, us: 1e3, ms: 1e6, s: 1e9 };
const fmt = (ns) => ns >= 1e6 ? (ns / 1e6).toFixed(3) + ' ms' : ns >= 1e3 ? (ns / 1e3).toFixed(3) + ' µs' : ns.toFixed(2) + ' ns';
const fmtPerOp = (ns) => {
  const perOp = ns / 4096;
  return perOp >= 1e3 ? (perOp / 1e3).toFixed(2) + ' µs' : perOp.toFixed(2) + ' ns';
};

for (const crate of ['world', 'mesh']) {
  const runs = [1, 2, 3].map((r) => {
    const lines = fs.readFileSync(`${dir}/${crate}-run${r}.txt`, 'utf8').split(/\r?\n/);
    const map = {};
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      let m = line.match(/^(\S+)\s+time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]/);
      if (m) {
        map[m[1]] = parseFloat(m[4]) * mult[m[5]];
        continue;
      }
      m = line.match(/^\s+time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]/);
      if (m && i > 0) {
        const name = lines[i - 1].trim();
        if (name && !name.startsWith('Benchmarking') && !name.startsWith('Found')) {
          map[name] = parseFloat(m[3]) * mult[m[4]];
        }
        continue;
      }
    }
    return map;
  });
  console.log(`\n===== ${crate} =====`);
  const names = Object.keys(runs[0]);
  console.log('| benchmark | run1 | run2 | run3 | median | spread | per-op (median) |');
  console.log('| --- | --- | --- | --- | --- | --- | --- |');
  for (const n of names) {
    const vals = runs.map((r) => r[n]);
    if (vals.some((v) => v === undefined)) { console.log(`| ${n} | MISSING | | | | | |`); continue; }
    const sorted = [...vals].sort((a, b) => a - b);
    const med = sorted[1];
    const spread = ((sorted[2] - sorted[0]) / med) * 100;
    console.log(`| ${n} | ${fmt(vals[0])} | ${fmt(vals[1])} | ${fmt(vals[2])} | ${fmt(med)} | ${spread.toFixed(1)}% | ${fmtPerOp(med)} |`);
  }
}
