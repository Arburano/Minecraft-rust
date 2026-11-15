# Estimates for PRD review. All inputs are assumptions marked below.
import math
SEC_PER_COL = 24          # world height 384 blocks (assumption, as modern MC)
def cols(r, shape="square"):
    return (2*r+1)**2 if shape=="square" else math.pi*r*r
print("== Columns / sections in view")
for r in (12, 32, 64):
    c=cols(r); print(f"r={r}: columns {c:.0f}, sections {c*SEC_PER_COL:.0f}")
print("\n== Client voxel+light RAM, no LOD (palette 4 bit = 2 KiB + light 2x2 KiB per non-uniform section)")
for r in (12, 64):
    s=cols(r)*SEC_PER_COL
    for frac in (0.4, 0.6):   # share of non-uniform sections (assumption)
        mb = s*frac*(2048+64+4096)/2**20
        print(f"r={r} nonuniform={frac:.0%}: {mb:.0f} MiB")
print("\n== Quad VRAM: 8 B/quad vs classic vertices")
for vb in (24, 32):
    classic = 4*vb + 6*4
    print(f"vertex {vb} B: {classic} B/quad with u32 indices -> {classic/8:.1f}x ; {4*vb} B w/o indices -> {4*vb/8:.0f}x")
print("\n== Sequential roadmap sum (stages 0-6), months")
lo=[2,6,4,4,6,3,6]; hi=[3,6,5,4,6,4,6]
print(sum(lo), sum(hi), f"= {sum(lo)/12:.1f}-{sum(hi)/12:.1f} years")
print("\n== Entity update bandwidth per player (estimate)")
ovh = 28+1+8+2+16   # IPv4+UDP 28, QUIC short hdr 1+CID 8+PN 2, AEAD 16 (assumptions)
for ents in (10, 20, 50):
    for hz in (20,):
        payload = ents*12  # 12 B per entity delta (assumption)
        bps = (payload+ovh+4)*hz*8
        print(f"{ents} entities @{hz}Hz, 12B each: {bps/1000:.0f} kbit/s")
print("\n== Join time: chunks to send at r=12")
for kb in (4, 8, 16):   # compressed KiB per column (assumption; replace with zstd bench)
    mb = cols(12)*kb/1024
    print(f"{kb} KiB/column -> {mb:.1f} MiB; at 20 Mbit/s {mb*8*1.048/20:.1f} s; at 50 Mbit/s {mb*8*1.048/50:.1f} s")
print("\n== WASM instances if one Store per region per mod")
for regions in (50,150):
    for mods in (20,50):
        for mem in (1,2):
            print(f"{regions} regions x {mods} mods x {mem} MiB = {regions*mods*mem/1024:.1f} GiB")
print("\n== LOD ring reach from 12-chunk LOD0, each ring 2x wider")
r=12; print([r*2**i for i in range(6)])
print("\n== texture layers: wgpu default max_texture_array_layers check needed")
