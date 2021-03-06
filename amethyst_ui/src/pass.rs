//! Simple flat forward drawing pass.

use std::cmp::{Ordering, PartialOrd};

use amethyst_assets::{AssetStorage, Loader};
use amethyst_renderer::{Encoder, Mesh, MeshHandle, PosTex, ScreenDimensions, Texture, VertexFormat};
use amethyst_renderer::error::Result;
use amethyst_renderer::pipe::{Effect, NewEffect};
use amethyst_renderer::pipe::pass::{Pass, PassApply, PassData, Supplier};
use cgmath::vec4;
use gfx::preset::blend;
use gfx::pso::buffer::ElemStride;
use gfx::state::ColorMask;
use hibitset::BitSet;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rayon::iter::internal::UnindexedConsumer;
use specs::{Entities, Entity, Fetch, Join, ReadStorage};

use super::*;

const VERT_SRC: &[u8] = include_bytes!("shaders/vertex.glsl");
const FRAG_SRC: &[u8] = include_bytes!("shaders/frag.glsl");

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)] // This is used by the shaders
#[repr(C)]
struct VertexArgs {
    proj_vec: [f32; 4],
    coord: [f32; 2],
    dimension: [f32; 2],
}

#[derive(Clone, Debug)]
struct CachedDrawOrder {
    pub cached: BitSet,
    pub cache: Vec<(f32, Entity)>,
}

/// Draw Ui elements, this uses target with name "amethyst_ui"
/// `V` is `VertexFormat`
#[derive(Clone, Debug)]
pub struct DrawUi {
    mesh_handle: MeshHandle,
    cached_draw_order: CachedDrawOrder,
}

impl DrawUi
where
    Self: Pass,
{
    /// Create instance of `DrawUi` pass
    pub fn new(loader: &Loader, mesh_storage: &AssetStorage<Mesh>) -> Self {
        // Initialize a single unit quad, we'll use this mesh when drawing quads later
        let data = vec![
            PosTex {
                position: [0., 1., 0.],
                tex_coord: [0., 0.],
            },
            PosTex {
                position: [1., 1., 0.],
                tex_coord: [1., 0.],
            },
            PosTex {
                position: [1., 0., 0.],
                tex_coord: [1., 1.],
            },
            PosTex {
                position: [0., 1., 0.],
                tex_coord: [0., 0.],
            },
            PosTex {
                position: [1., 0., 0.],
                tex_coord: [1., 1.],
            },
            PosTex {
                position: [0., 0., 0.],
                tex_coord: [0., 1.],
            },
        ].into();
        let mesh_handle = loader.load_from_data(data, (), mesh_storage);
        DrawUi {
            mesh_handle,
            cached_draw_order: CachedDrawOrder {
                cached: BitSet::new(),
                cache: Vec::new(),
            },
        }
    }
}

impl<'a> PassData<'a> for DrawUi {
    type Data = (
        Entities<'a>,
        Fetch<'a, ScreenDimensions>,
        Fetch<'a, AssetStorage<Mesh>>,
        Fetch<'a, AssetStorage<Texture>>,
        ReadStorage<'a, UiImage>,
        ReadStorage<'a, UiTransform>,
        ReadStorage<'a, UiText>,
    );
}

impl<'a> PassApply<'a> for DrawUi {
    type Apply = DrawUiApply<'a>;
}

impl Pass for DrawUi {
    fn compile(&self, effect: NewEffect) -> Result<Effect> {
        use std::mem;
        effect
            .simple(VERT_SRC, FRAG_SRC)
            .with_raw_constant_buffer("VertexArgs", mem::size_of::<VertexArgs>(), 1)
            .with_raw_vertex_buffer(PosTex::ATTRIBUTES, PosTex::size() as ElemStride, 0)
            .with_texture("albedo")
            .with_blended_output("color", ColorMask::all(), blend::ALPHA, None)
            .build()
    }

    fn apply<'a, 'b: 'a>(
        &'a mut self,
        supplier: Supplier<'a>,
        (entities, screen_dimensions, mesh_storage, tex_storage, ui_image, ui_transform, ui_text): (
            Entities<'a>,
            Fetch<'a, ScreenDimensions>,
            Fetch<'a, AssetStorage<Mesh>>,
            Fetch<'a, AssetStorage<Texture>>,
            ReadStorage<'a, UiImage>,
            ReadStorage<'a, UiTransform>,
            ReadStorage<'a, UiText>,
        ),
) -> DrawUiApply<'a>{
        DrawUiApply {
            entities,
            screen_dimensions,
            mesh_storage,
            tex_storage,
            ui_image,
            ui_transform,
            ui_text,
            unit_mesh: self.mesh_handle.clone(),
            cached_draw_order: &mut self.cached_draw_order,
            supplier,
        }
    }
}

pub struct DrawUiApply<'a> {
    entities: Entities<'a>,
    screen_dimensions: Fetch<'a, ScreenDimensions>,
    mesh_storage: Fetch<'a, AssetStorage<Mesh>>,
    tex_storage: Fetch<'a, AssetStorage<Texture>>,
    ui_image: ReadStorage<'a, UiImage>,
    ui_transform: ReadStorage<'a, UiTransform>,
    ui_text: ReadStorage<'a, UiText>,
    unit_mesh: MeshHandle,
    cached_draw_order: &'a mut CachedDrawOrder,
    supplier: Supplier<'a>,
}

impl<'a> ParallelIterator for DrawUiApply<'a> {
    type Item = ();

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        let DrawUiApply {
            entities,
            screen_dimensions,
            mesh_storage,
            tex_storage,
            ui_image,
            ui_transform,
            ui_text,
            unit_mesh,
            cached_draw_order,
            supplier,
            ..
        } = self;

        let entities = &*entities;
        let screen_dimensions = &screen_dimensions;
        let mesh_storage = &mesh_storage;
        let tex_storage = &tex_storage;
        let ui_image = &ui_image;
        let ui_text = &ui_text;
        let ui_transform = &ui_transform;
        let unit_mesh = &unit_mesh;

        // Populate and update the draw order cache.
        {
            let bitset = &mut cached_draw_order.cached;
            cached_draw_order.cache.retain(|&(_z, entity)| {
                let keep = ui_transform.get(entity).is_some();
                if !keep {
                    bitset.remove(entity.id());
                }
                keep
            });
        }


        for &mut (ref mut z, entity) in &mut cached_draw_order.cache {
            *z = ui_transform.get(entity).unwrap().z;
        }

        // Attempt to insert the new entities in sorted position.  Should reduce work during
        // the sorting step.
        let transform_set = ui_transform.check();
        {
            // Create a bitset containing only the new indices.
            let new = (&transform_set ^ &cached_draw_order.cached) & &transform_set;
            for (entity, transform, _new) in (entities, ui_transform, &new).join() {
                let pos = cached_draw_order
                    .cache
                    .iter()
                    .position(|&(cached_z, _)| transform.z >= cached_z);
                match pos {
                    Some(pos) => cached_draw_order.cache.insert(pos, (transform.z, entity)),
                    None => cached_draw_order.cache.push((transform.z, entity)),
                }
            }
        }
        cached_draw_order.cached = transform_set;

        // Sort from largest z value to smallest z value.
        // Most of the time this shouldn't do anything but you still need it for if the z values
        // change.
        cached_draw_order
            .cache
            .sort_unstable_by(|&(z1, _), &(z2, _)| {
                z2.partial_cmp(&z1).unwrap_or(Ordering::Equal)
            });

        //let cached_draw_order = &cached_draw_order;

        let proj_vec = vec4(
            2. / screen_dimensions.width(),
            -2. / screen_dimensions.height(),
            -2.,
            1.,
        );

        let cached_draw_order = &*cached_draw_order.cache;

        // This pass can't be executed in parallel, so we use a dumby bitset of a
        // single element to provide a fake parallel iterator that performs the entire
        // pass in the first iteration.
        supplier
            .supply((0..1).into_par_iter().map(move |_id| {
                move |encoder: &mut Encoder, effect: &mut Effect| for &(_z, entity) in
                    cached_draw_order
                {
                    // This won't panic as we guaranteed earlier these entities are present.
                    let ui_transform = ui_transform.get(entity).unwrap();
                    let mesh = match mesh_storage.get(unit_mesh) {
                        Some(mesh) => mesh,
                        None => return,
                    };
                    let vbuf = match mesh.buffer(PosTex::ATTRIBUTES) {
                        Some(vbuf) => vbuf.clone(),
                        None => continue,
                    };
                    let vertex_args = VertexArgs {
                        proj_vec: proj_vec.into(),
                        coord: [ui_transform.x, ui_transform.y],
                        dimension: [ui_transform.width, ui_transform.height],
                    };
                    effect.update_constant_buffer("VertexArgs", &vertex_args, encoder);
                    effect.data.vertex_bufs.push(vbuf);
                    if let Some(image) = ui_image
                        .get(entity)
                        .and_then(|image| tex_storage.get(&image.texture))
                    {
                        effect.data.textures.push(image.view().clone());
                        effect.data.samplers.push(image.sampler().clone());
                        effect.draw(mesh.slice(), encoder);
                        effect.clear();
                    }

                    if let Some(image) = ui_text
                        .get(entity)
                        .and_then(|ref ui_text| ui_text.texture.as_ref())
                        .and_then(|texture| tex_storage.get(texture))
                    {
                        effect.data.textures.push(image.view().clone());
                        effect.data.samplers.push(image.sampler().clone());
                        effect.draw(mesh.slice(), encoder);
                        effect.clear();
                    }
                }
            }))
            .drive_unindexed(consumer)
    }
}
