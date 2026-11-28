//! T1-25: light indirection changes reach the GPU once per frame, one write
//! per changed z slice, not one per section. Needs a GPU adapter (software
//! Vulkan is enough); skips without one unless `REQUIRE_GPU=1`.

use glam::IVec3;
use light::{Nibbles, SectionLight};
use render::terrain::renderer::{TerrainPath, TerrainRenderer, TerrainTextures};

#[test]
fn a_row_of_removed_sections_is_one_write_per_slice() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Some((device, queue)) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()
            .and_then(|a| {
                pollster::block_on(a.request_device(&wgpu::DeviceDescriptor::default())).ok()
            })
    else {
        assert!(std::env::var_os("REQUIRE_GPU").is_none(), "no GPU adapter");
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let levels: Vec<Vec<u8>> = (0..5)
        .map(|m| [128u8; 4].repeat((16 >> m) * (16 >> m)))
        .collect();
    let mut r = TerrainRenderer::new(
        &device,
        &queue,
        TerrainPath::Pulling,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &TerrainTextures {
            size: 16,
            layers: 1,
            levels: &levels,
        },
        &[0u32; 12],
        1 << 12,
    );
    let lit = SectionLight {
        block: Nibbles::Single(3),
        sky: Nibbles::Single(15),
    };
    // A row of 39 columns at z = 7 (one z slice) and a row along z at
    // x = 2 crossing 10 slices.
    let along_x: Vec<IVec3> = (0..39)
        .flat_map(|x| (-4..20).map(move |y| IVec3::new(x, y, 7)))
        .collect();
    let along_z: Vec<IVec3> = (10..20).map(|z| IVec3::new(2, 0, z)).collect();
    for &s in along_x.iter().chain(&along_z) {
        r.set_section_light(&queue, s, &lit).expect("light");
    }
    assert_eq!(r.flush_light_index(&queue), 11, "1 slice + 10 slices");
    assert_eq!(r.flush_light_index(&queue), 0, "nothing changed since");
    for &s in &along_x {
        r.remove_section_light(&queue, s);
    }
    assert_eq!(r.flush_light_index(&queue), 1, "936 removals, one write");
    // Removing again changes nothing: no write.
    r.remove_section_light(&queue, along_x[0]);
    assert_eq!(r.flush_light_index(&queue), 0);
}
