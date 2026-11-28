//! The world on screen: the T1-11 terrain renderer fed from a session, with
//! block textures (own installation or fallback), cave culling and the camera.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use client::session::Session;
use client::textures::BlockTextures;
use glam::{IVec3, Vec3};
use render::terrain::camera::{look_to, perspective};
use render::terrain::culling::{Frustum, cave_visible};
use render::terrain::profiles::RenderSettings;
use render::terrain::renderer::{
    CameraParams, FrameStats, TerrainPath, TerrainRenderer, TerrainTextures,
};

/// Quads the shared buffer holds (8 bytes each, 64 MiB): enough for 16 chunks.
const QUAD_CAPACITY: u32 = 8 << 20;

/// Sky color (linear RGB), also the fog color and the clear color.
pub const SKY: [f32; 3] = [0.55, 0.72, 0.95];

/// GPU state of the world being played.
pub struct WorldView {
    terrain: TerrainRenderer,
    visible: HashSet<IVec3>,
    upload_errors: u32,
}

/// What one [`WorldView::sync`] uploaded and how long it took.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SyncCost {
    /// Collecting re-packed batches and writing them to the GPU.
    pub batch_time: Duration,
    /// Light atlas removals and writes.
    pub light_time: Duration,
    /// Batches written (an empty one removes a batch).
    pub batches: u32,
    /// Quads in those batches.
    pub quads: u32,
    /// Sections whose light was written.
    pub light_sections: u32,
    /// Sections whose light was dropped (their column unloaded).
    pub removed_sections: u32,
}

impl WorldView {
    /// Creates the renderer with the block textures built for this world.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: TerrainPath,
        format: wgpu::TextureFormat,
        textures: &BlockTextures,
    ) -> Self {
        let layer_table = &textures.layer_table;
        let empty: Vec<Vec<u8>> = Vec::new();
        let (size, layers, levels) = textures
            .set
            .arrays()
            .first()
            .map_or((16, 0, empty.as_slice()), |a| {
                (a.size(), a.layers() as u32, a.levels())
            });
        let terrain = TerrainRenderer::new(
            device,
            queue,
            path,
            format,
            &TerrainTextures {
                size,
                layers,
                levels,
            },
            layer_table,
            QUAD_CAPACITY,
        );
        Self {
            terrain,
            visible: HashSet::new(),
            upload_errors: 0,
        }
    }

    /// Uploads what changed in the session since the last frame and reports
    /// how much it was (T1-25 spike attribution).
    pub fn sync(&mut self, queue: &wgpu::Queue, session: &mut Session) -> SyncCost {
        let t0 = Instant::now();
        let mut cost = SyncCost::default();
        let (terrain, errors) = (&mut self.terrain, &mut self.upload_errors);
        let mut pending_light = Vec::new();
        let mut removed = Vec::new();
        let (batches, quads) = (&mut cost.batches, &mut cost.quads);
        session.render_updates(
            |sp| removed.push(IVec3::new(sp.x, sp.y, sp.z)),
            |pos, packed| {
                *batches += 1;
                *quads += packed.len() as u32;
                if terrain.upload_batch(queue, pos, packed).is_err() {
                    *errors += 1;
                }
            },
            |sp, l| pending_light.push((IVec3::new(sp.x, sp.y, sp.z), l.clone())),
        );
        let t1 = Instant::now();
        cost.batch_time = t1 - t0;
        cost.removed_sections = removed.len() as u32;
        cost.light_sections = pending_light.len() as u32;
        // Free unloaded sections' bricks before new light takes slots.
        for sp in removed {
            self.terrain.remove_section_light(queue, sp);
        }
        for (sp, l) in pending_light {
            if self.terrain.set_section_light(queue, sp, &l).is_err() {
                self.upload_errors += 1;
            }
        }
        cost.light_time = t1.elapsed();
        cost
    }

    /// Replaces the block textures (resource packs changed).
    pub fn set_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &BlockTextures,
    ) {
        let empty: Vec<Vec<u8>> = Vec::new();
        let (size, layers, levels) = textures
            .set
            .arrays()
            .first()
            .map_or((16, 0, empty.as_slice()), |a| {
                (a.size(), a.layers() as u32, a.levels())
            });
        self.terrain.set_textures(
            device,
            queue,
            &TerrainTextures {
                size,
                layers,
                levels,
            },
            &textures.layer_table,
        );
    }

    /// Uses a shader pack's terrain hooks (`None` = built-in); errors leave
    /// the current look in place.
    pub fn set_shader(&mut self, device: &wgpu::Device, shade: Option<&str>) -> Result<(), String> {
        self.terrain.set_shader_pack(device, shade)
    }

    /// Shader pack option values and time.
    pub fn set_pack_params(&self, queue: &wgpu::Queue, options: &[f32; 8], time: f32) {
        self.terrain.set_pack_params(queue, options, time);
    }

    /// Culls and prepares a frame; returns the camera used.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        session: &Session,
        settings: &RenderSettings,
        fov_degrees: f32,
        aspect: f32,
        alpha: f64,
    ) -> FrameStats {
        let (eye, dir) = session.camera(alpha);
        let (eye, dir) = (eye.as_vec3(), dir.as_vec3());
        let far = settings.render_distance as f32 * 16.0;
        let view_proj = perspective(fov_degrees.to_radians(), aspect, 0.1, far + 32.0)
            * look_to(eye, dir, Vec3::Y);
        let camera = CameraParams {
            view_proj,
            position: eye,
            daylight: 1.0,
            fog: (far * settings.fog_start, far),
            sky: SKY,
        };
        let visible = if settings.cave_culling {
            let vis: HashMap<IVec3, mesh::ChunkVisibility> = session
                .loader()
                .visibility()
                .iter()
                .map(|(m, v)| (IVec3::new(m.x, m.y, m.z), *v))
                .collect();
            let frustum = Frustum::from_view_proj(view_proj);
            let cam = (eye / 32.0).floor().as_ivec3();
            let radius = (far / 32.0).ceil() as i32 + 1;
            self.visible.clear();
            for c in cave_visible(cam, &vis, Some(&frustum), radius, 32.0) {
                self.visible.insert(c.div_euclid(IVec3::splat(2)));
            }
            Some(&self.visible)
        } else {
            None
        };
        self.terrain.prepare(queue, &camera, visible)
    }

    /// Records the terrain into a pass with the renderer's depth format.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.terrain.render(pass);
    }

    /// Batches or light sections that did not fit (shown in the debug overlay).
    #[must_use]
    pub fn upload_errors(&self) -> u32 {
        self.upload_errors
    }
}
