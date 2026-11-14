//! The material every wall, floor and ceiling is drawn with.
//!
//! It is Bevy's standard physically-based material with one change: instead
//! of reading the surface colour from a texture, `surface.wesl` computes it
//! from the pixel's position in the world. Lighting, fog and tonemapping are
//! still Bevy's.

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::space::RenderOrigin;

/// `ExtendedMaterial<Base, Extension>` is Bevy's way of customising a
/// material without rewriting it: the base supplies everything, and the
/// extension overrides the fragment shader.
pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceExtension>;

/// Data handed to the shader. `AsBindGroup` generates the code that uploads
/// it to the GPU; binding 100 keeps clear of the slots the standard material
/// uses.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct SurfaceExtension {
    #[uniform(100)]
    pub uniform: SurfaceUniform,
}

/// Must match `struct Surface` in the shader, field for field.
#[derive(ShaderType, Debug, Clone, Copy)]
pub struct SurfaceUniform {
    pub origin: IVec2,
    pub ceiling: f32,
    pub pad: f32,
}

impl MaterialExtension for SurfaceExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://backrooms_game/surface.wesl".into()
    }
}

/// The one surface material shared by every chunk.
#[derive(Resource)]
pub struct SurfaceHandle(pub Handle<SurfaceMaterial>);

pub struct SurfacePlugin {
    pub ceiling: f32,
}

impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        // Compiles the shader source into the binary, so the game is a single
        // file with no assets folder to lose. It is then loadable as
        // `embedded://<crate name>/<path under src>`.
        embedded_asset!(app, "surface.wesl");
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default());

        let origin = *app.world().resource::<RenderOrigin>();
        let material = SurfaceMaterial {
            base: StandardMaterial { reflectance: 0.25, ..default() },
            extension: SurfaceExtension {
                uniform: SurfaceUniform {
                    origin: IVec2::new(origin.cell.x, origin.cell.y),
                    ceiling: self.ceiling,
                    pad: 0.0,
                },
            },
        };
        let handle = app.world_mut().resource_mut::<Assets<SurfaceMaterial>>().add(material);
        app.insert_resource(SurfaceHandle(handle));
    }
}
