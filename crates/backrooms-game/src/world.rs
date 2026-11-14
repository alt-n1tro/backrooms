//! Keeps the chunks around the player on screen.
//!
//! Building a chunk (generating it, baking its light, meshing it) happens on
//! worker threads. The main thread only decides what is wanted, hands out
//! the jobs and, when one comes back, uploads the finished mesh. Walking
//! therefore never waits on chunk building.
//!
//! This module also keeps the render origin near the player; see
//! [`recentre`].

use backrooms_core::coord::{CHUNK_SHIFT, CHUNK_SIZE};
use backrooms_core::{ChunkPos, LightState};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};

use crate::bake::Baker;
use crate::lights::{LightKind, PanelMaterials, spawn_fixture};
use crate::mesh::{Geometry, chunk_geometry};
use crate::player::Player;
use crate::space::{CELL, ChunkCache, LocalPlan, RenderOrigin, plan_to_world, world_to_plan};
use crate::surface::{SurfaceHandle, SurfaceMaterial};

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

/// How many chunks the player may stray from the render origin before the
/// origin is moved to them.
#[derive(Resource, Clone, Copy)]
pub struct RecentreAfter(pub i32);

/// Marks everything whose `Transform` is a position in the world, and so
/// must move when the render origin does.
#[derive(Component)]
pub struct Anchored;

/// The chunk entities currently spawned.
#[derive(Resource, Default)]
pub struct LoadedChunks(HashMap<ChunkPos, Entity>);

/// Chunks being built on worker threads. Dropping a `Task` cancels it, so
/// removing an entry here abandons that chunk.
#[derive(Resource, Default)]
struct Building(HashMap<ChunkPos, Task<Built>>);

/// Everything a worker produces for one chunk.
struct Built {
    geometry: Geometry,
    /// Panel centre (metres from the chunk corner, at ceiling height) and
    /// what the core says about the fixture.
    fixtures: Vec<(Vec3, LightState, u32)>,
}

/// Most chunks under construction at once. Enough to keep every core busy
/// without queueing work for places the player may have already left.
const MAX_BUILDING: usize = 16;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedChunks>()
            .init_resource::<Building>()
            // Re-centring must see where the player ended up this frame, and
            // streaming must see the origin re-centring settled on.
            .add_systems(Update, (recentre, stream_chunks).chain().after(crate::player::walk));
    }
}

/// Signed distance in chunks from `from` to `to`, the short way round the
/// torus.
fn chunk_delta(from: ChunkPos, to: ChunkPos) -> IVec2 {
    let (dx, dy) = from.origin().delta_to(to.origin());
    IVec2::new(dx >> CHUNK_SHIFT, dy >> CHUNK_SHIFT)
}

/// The whole job for one chunk. Runs on a worker thread, so it takes only
/// owned or shared-immutable data.
fn build_chunk(cache: &ChunkCache, pos: ChunkPos, ceiling: f32) -> Built {
    let plan = LocalPlan::around(cache, pos);
    let baker = Baker::new(&plan, &plan.fixtures, ceiling);
    let geometry = chunk_geometry(&plan, &baker, ceiling);
    let fixtures = cache
        .get(pos)
        .fixtures()
        .iter()
        .map(|f| {
            // The panel covers this cell and the one east of it, so its
            // centre is one cell east and half a cell north of the corner.
            let middle = Vec2::new(f.x as f32 + 1.0, f.y as f32 + 0.5) * CELL;
            (plan_to_world(middle, ceiling), f.state, f.seed)
        })
        .collect();
    Built { geometry, fixtures }
}

/// Keeps the render origin within a few chunks of the player.
///
/// `f32` positions lose precision as they grow: a kilometre from zero the
/// smallest step is already a tenth of a millimetre, and it doubles with
/// every doubling of distance. So when the player has wandered far enough,
/// the origin is moved to their chunk and everything in the scene is shifted
/// back by the same amount. Nothing visibly moves. The shift is a whole
/// number of chunks, which keeps every pattern in the surface shader lined
/// up.
fn recentre(
    mut origin: ResMut<RenderOrigin>,
    limit: Res<RecentreAfter>,
    surface: Res<SurfaceHandle>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    mut anchored: Query<&mut Transform, With<Anchored>>,
    player: Single<Entity, With<Player>>,
) {
    let Ok(player_at) = anchored.get(*player).map(|t| t.translation) else {
        return;
    };
    let here = origin.cell_at(world_to_plan(player_at)).chunk();
    let strayed = chunk_delta(origin.cell.chunk(), here);
    if strayed.abs().max_element() < limit.0 {
        return;
    }

    let shift = plan_to_world(origin.plan_of(here.origin()), 0.0);
    origin.cell = here.origin();
    for mut transform in &mut anchored {
        transform.translation -= shift;
    }
    // The shader adds the origin's cell to every position, so it must hear
    // about the move in the same frame.
    if let Some(mut material) = materials.get_mut(&surface.0) {
        material.extension.uniform.origin = IVec2::new(origin.cell.x, origin.cell.y);
    }
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
    cache: Res<ChunkCache>,
    mut loaded: ResMut<LoadedChunks>,
    mut building: ResMut<Building>,
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
    building.0.retain(|&pos, _| in_range(chunk_delta(centre, pos), radius + 1));

    // What is wanted and neither on screen nor on its way, nearest first.
    let mut missing: Vec<IVec2> = (-radius..=radius)
        .flat_map(|dy| (-radius..=radius).map(move |dx| IVec2::new(dx, dy)))
        .filter(|&d| in_range(d, radius))
        .filter(|&d| {
            let pos = centre.offset(d.x, d.y);
            !loaded.0.contains_key(&pos) && !building.0.contains_key(&pos)
        })
        .collect();
    missing.sort_by_key(|d| d.length_squared());

    let mut finished: Vec<(ChunkPos, Built)> = Vec::new();

    // On the very first frame the surroundings are built right here, so the
    // game never opens onto empty space.
    let first_frame = loaded.0.is_empty() && building.0.is_empty();
    if first_frame {
        let near = missing.iter().filter(|d| d.abs().max_element() <= 2);
        finished.extend(near.map(|d| {
            let pos = centre.offset(d.x, d.y);
            (pos, build_chunk(&cache, pos, ceiling.0))
        }));
        missing.retain(|d| d.abs().max_element() > 2);
    }

    // Hand out jobs. `spawn` puts the future on Bevy's pool of worker
    // threads and returns at once with a `Task`, a handle to the result.
    // `async move` makes the block own what it uses: the cache is a cheap
    // shared pointer, so each task gets its own clone.
    let pool = AsyncComputeTaskPool::get();
    let free = MAX_BUILDING.saturating_sub(building.0.len());
    for d in missing.into_iter().take(free) {
        let pos = centre.offset(d.x, d.y);
        let (cache, ceiling) = (cache.clone(), ceiling.0);
        building.0.insert(pos, pool.spawn(async move { build_chunk(&cache, pos, ceiling) }));
    }

    // Collect whatever the workers have finished. `check_ready` looks
    // without waiting.
    building.0.retain(|&pos, task| match check_ready(task) {
        Some(built) => {
            finished.push((pos, built));
            false
        }
        None => true,
    });
    // While nothing at all is on screen, wait for the first result rather
    // than draw an empty frame.
    if loaded.0.is_empty()
        && finished.is_empty()
        && let Some(pos) = building.0.keys().next().copied()
    {
        let task = building.0.remove(&pos).expect("key was just read");
        finished.push((pos, block_on(task)));
    }

    for (pos, built) in finished {
        let corner = origin.plan_of(pos.origin());
        let entity = commands
            .spawn((
                Anchored,
                Mesh3d(meshes.add(built.geometry.into_mesh())),
                MeshMaterial3d(surface.0.clone()),
                Transform::from_translation(plan_to_world(corner, 0.0)),
            ))
            .id();
        for (local, state, seed) in built.fixtures {
            spawn_fixture(&mut commands, entity, local, state, seed, *kind, &panels);
        }
        loaded.0.insert(pos, entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backrooms_core::{GeneratorVersion, LevelId};

    #[test]
    fn chunk_delta_takes_the_short_way_round() {
        let a = ChunkPos::new(ChunkPos::MAX, 0);
        let b = ChunkPos::new(ChunkPos::MIN + 1, -3);
        assert_eq!(chunk_delta(a, b), IVec2::new(2, -3));
        assert_eq!(chunk_delta(b, a), IVec2::new(-2, 3));
        assert_eq!(chunk_delta(a, a), IVec2::ZERO);
    }

    #[test]
    fn building_a_chunk_is_repeatable() {
        // Workers finish in any order; what they produce must not depend on
        // it, nor on what happens to be in the cache.
        let pos = ChunkPos::new(12, -7);
        let warm = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 256);
        let first = build_chunk(&warm, pos, 2.6);
        let again = build_chunk(&warm, pos, 2.6);
        let cold = build_chunk(&ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 16), pos, 2.6);
        for other in [&again, &cold] {
            assert_eq!(first.geometry.positions, other.geometry.positions);
            assert_eq!(first.geometry.colors, other.geometry.colors);
            assert_eq!(first.geometry.indices, other.geometry.indices);
            assert_eq!(first.fixtures.len(), other.fixtures.len());
        }
        assert_eq!(first.fixtures.len(), warm.get(pos).fixtures().len());
    }
}
