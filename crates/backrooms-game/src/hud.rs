//! The address readout in the top-left corner.

use backrooms_core::{Address, level_info, zstack};
use bevy::prelude::*;

use crate::player::Player;
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

fn update(
    origin: Res<RenderOrigin>,
    player: Single<&Transform, With<Player>>,
    mut text: Single<&mut Text, With<Readout>>,
) {
    let wanted = readout(&address_at(&origin, player.translation));
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
    fn address_tracks_the_player_across_the_origin() {
        let origin = RenderOrigin { version: GeneratorVersion::V1, level: LevelId::ZERO, cell: CellPos::new(160, 320) };
        // 0.6 m west and 1.1 m north of the origin corner; north is -Z.
        let address = address_at(&origin, Vec3::new(-0.6, 1.6, -1.1));
        assert_eq!(address.cell, CellPos::new(158, 322));
    }
}
