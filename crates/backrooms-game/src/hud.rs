//! The address readout in the top-left corner.

use backrooms_core::{Address, level_info, zstack};
use bevy::prelude::*;

use crate::player::Player;
use crate::screen::Screen;
use crate::space::{RenderOrigin, world_to_plan};

#[derive(Component)]
struct Readout;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, update);
    }
}

/// The address of the cell under a world position.
pub fn address_at(origin: &RenderOrigin, world: Vec3) -> Address {
    Address { version: origin.version, level: origin.level, cell: origin.cell_at(world_to_plan(world)) }
}

/// `v1 | LEVEL 0 | X 12 | Y -5 | Z 57`
///
/// The design calls for a middle dot between fields, but Bevy's built-in font
/// has no such glyph. A bar stands in until the game ships its own font.
pub fn readout(address: &Address) -> String {
    let info = level_info(address.version, address.level);
    format!(
        "v{} | LEVEL {} | X {} | Y {} | Z {}",
        address.version.number(),
        info.name,
        address.cell.x,
        address.cell.y,
        zstack::depth_of(address.version, address.level)
    )
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Readout,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(0.95, 0.93, 0.8, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(10), left: px(12), ..default() },
    ));
}

/// The F3 line: frame rate and the size the 3D picture is drawn at.
pub fn stats(frame_seconds: f32, picture: UVec2, window: UVec2) -> String {
    let fps = if frame_seconds > 0.0 { (1.0 / frame_seconds).round() } else { 0.0 };
    let percent = (picture.x as f32 / window.x.max(1) as f32 * 100.0).round();
    format!(
        "{fps} fps | {:.1} ms | drawn at {} x {} ({percent}%) | {}",
        frame_seconds * 1000.0,
        picture.x,
        picture.y,
        env!("CARGO_PKG_VERSION")
    )
}

/// What the HUD remembers between frames. `Local<T>` gives a system private
/// state that lives as long as the app; grouping it in one struct keeps the
/// parameter list short.
#[derive(Default)]
struct StatsState {
    visible: bool,
    /// Running average of the frame time, in seconds.
    average: f32,
    /// The value currently on screen, refreshed a few times a second so the
    /// number can be read rather than flickering.
    shown: f32,
    since_refresh: f32,
}

// A system's parameters are its declared inputs, so a long list here is
// information rather than clutter.
#[allow(clippy::too_many_arguments)]
fn update(
    origin: Res<RenderOrigin>,
    screen: Res<Screen>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    player: Single<&Transform, With<Player>>,
    mut text: Single<&mut Text, With<Readout>>,
    mut state: Local<StatsState>,
) {
    // Many laptops need Fn held for the F-keys, so the key left of 1 works
    // as well.
    if keys.any_just_pressed([KeyCode::F3, KeyCode::Backquote]) {
        state.visible = !state.visible;
    }
    let dt = time.delta_secs();
    state.average += (dt - state.average) * 0.05;
    state.since_refresh += dt;
    if state.since_refresh > 0.25 {
        state.since_refresh = 0.0;
        state.shown = state.average;
    }

    let mut wanted = readout(&address_at(&origin, player.translation));
    if state.visible {
        wanted.push('\n');
        wanted.push_str(&stats(state.shown, screen.size, window.physical_size()));
    }
    // Comparing first avoids re-laying-out the text on frames where the
    // player has not crossed into another cell.
    if text.0 != wanted {
        text.0 = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backrooms_core::{CellPos, GeneratorVersion, LevelId};

    #[test]
    fn readout_follows_the_spec() {
        let address = Address { version: GeneratorVersion::V1, level: LevelId::ZERO, cell: CellPos::new(12, -5) };
        let depth = zstack::depth_of(GeneratorVersion::V1, LevelId::ZERO);
        assert_eq!(readout(&address), format!("v1 | LEVEL 0 | X 12 | Y -5 | Z {depth}"));
    }

    #[test]
    fn stats_line_reads_naturally() {
        let line = stats(1.0 / 60.0, UVec2::new(1832, 1145), UVec2::new(2880, 1800));
        assert_eq!(line, concat!("60 fps | 16.7 ms | drawn at 1832 x 1145 (64%) | ", env!("CARGO_PKG_VERSION")));
        // Before the first frame there is nothing to divide by.
        assert!(stats(0.0, UVec2::ONE, UVec2::ZERO).starts_with("0 fps"));
    }

    #[test]
    fn address_tracks_the_player_across_the_origin() {
        let origin = RenderOrigin { version: GeneratorVersion::V1, level: LevelId::ZERO, cell: CellPos::new(160, 320) };
        // 0.6 m west and 1.1 m north of the origin corner; north is -Z.
        let address = address_at(&origin, Vec3::new(-0.6, 1.6, -1.1));
        assert_eq!(address.cell, CellPos::new(158, 322));
    }
}
