//! Keeps the chunks around the player on screen.
//!
//! Milestone 2 keeps this deliberately simple: chunks within a radius are
//! generated and meshed on the main thread, a few per frame, and dropped when
//! the player walks away. Background threads, an LRU cache and occlusion
//! culling are milestone 3.

use backrooms_core::ChunkPos;
use backrooms_core::coord::{CHUNK_SHIFT, CHUNK_SIZE};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::lights::{LightKind, PanelMaterials, spawn_fixture};
use crate::mesh::chunk_geometry;
use crate::player::Player;
use crate::space::{CELL, ChunkStore, RenderOrigin, plan_to_world, world_to_plan};
use crate::surface::SurfaceHandle;

/// How far the world is drawn, in chunks (8 m each).
#[derive(Resource, Clone, Copy)]
pub struct ViewDistance(pub i32);

impl ViewDistance {
    pub fn metres(self) -> f32 {
        self.0 as f32 * CHUNK_SIZE as f32 * CELL
    }
}

/// Floor-to-ceiling height of the level, in metres.
#[derive(Resource, Clone, Copy)]
pub struct Ceiling(pub f32);

/// The chunk entities currently spawned.
#[derive(Resource, Default)]
pub struct LoadedChunks(HashMap<ChunkPos, Entity>);

/// Most chunks built in one frame once the first view is up. Each costs well
/// under a millisecond, so this keeps a running player from causing a hitch.
const PER_FRAME: usize = 6;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedChunks>().add_systems(Update, stream_chunks);
    }
}

/// Signed distance in chunks from `from` to `to`, the short way round the
/// torus.
fn chunk_delta(from: ChunkPos, to: ChunkPos) -> IVec2 {
    let (dx, dy) = from.origin().delta_to(to.origin());
    IVec2::new(dx >> CHUNK_SHIFT, dy >> CHUNK_SHIFT)
}

// Bevy systems take what they need as parameters, and the scheduler supplies
// them: `Res` is shared access to a global resource, `ResMut` exclusive
// access, `Single` the one entity matching a query, and `Commands` a queue of
// world changes applied after the system returns. A streaming system touches
// many things, hence the long list.
#[allow(clippy::too_many_arguments)]
fn stream_chunks(
    mut commands: Commands,
    player: Single<&Transform, With<Player>>,
    origin: Res<RenderOrigin>,
    view: Res<ViewDistance>,
    ceiling: Res<Ceiling>,
    kind: Res<LightKind>,
    surface: Res<SurfaceHandle>,
    panels: Res<PanelMaterials>,
    mut store: ResMut<ChunkStore>,
    mut loaded: ResMut<LoadedChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let centre = origin.cell_at(world_to_plan(player.translation)).chunk();
    let radius = view.0;
    let in_range = |d: IVec2, r: i32| d.length_squared() <= r * r + r;

    // Drop what has fallen out of range. The margin of one chunk stops a
    // player pacing across a boundary from loading and unloading every step.
    loaded.0.retain(|&pos, &mut entity| {
        let keep = in_range(chunk_delta(centre, pos), radius + 1);
        if !keep {
            commands.entity(entity).despawn();
        }
        keep
    });
    store.retain(|pos| in_range(chunk_delta(centre, pos), radius + 3));

    // Build what is missing, nearest first.
    let mut missing: Vec<IVec2> = (-radius..=radius)
        .flat_map(|dy| (-radius..=radius).map(move |dx| IVec2::new(dx, dy)))
        .filter(|&d| in_range(d, radius) && !loaded.0.contains_key(&centre.offset(d.x, d.y)))
        .collect();
    missing.sort_by_key(|d| d.length_squared());
    // The very first frame builds everything, so the game never opens onto
    // empty space.
    let budget = if loaded.0.is_empty() { usize::MAX } else { PER_FRAME };

    for d in missing.into_iter().take(budget) {
        let pos = centre.offset(d.x, d.y);
        // The mesher looks one cell past the chunk on every side.
        store.ensure_around(pos);
        let geometry = chunk_geometry(&store.plan(pos.origin()), ceiling.0);
        let corner = origin.plan_of(pos.origin());

        let entity = commands
            .spawn((
                Mesh3d(meshes.add(geometry.into_mesh())),
                MeshMaterial3d(surface.0.clone()),
                Transform::from_translation(plan_to_world(corner, 0.0)),
            ))
            .id();

        let chunk = store.get(pos).expect("ensured above");
        for fixture in chunk.fixtures() {
            // The panel covers this cell and the one east of it, so its
            // centre is one cell east and half a cell north of the corner.
            let middle = Vec2::new(fixture.x as f32 + 1.0, fixture.y as f32 + 0.5) * CELL;
            let local = plan_to_world(middle, ceiling.0);
            spawn_fixture(&mut commands, entity, local, fixture.state, fixture.seed, *kind, &panels);
        }
        loaded.0.insert(pos, entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_delta_takes_the_short_way_round() {
        let a = ChunkPos::new(ChunkPos::MAX, 0);
        let b = ChunkPos::new(ChunkPos::MIN + 1, -3);
        assert_eq!(chunk_delta(a, b), IVec2::new(2, -3));
        assert_eq!(chunk_delta(b, a), IVec2::new(-2, 3));
        assert_eq!(chunk_delta(a, a), IVec2::ZERO);
    }
}
