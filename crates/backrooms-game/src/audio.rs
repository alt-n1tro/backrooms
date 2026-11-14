//! The fluorescent hum.
//!
//! Synthesised sample by sample, with no audio files. A fluorescent fixture
//! hums at twice the mains frequency (the ballast flexes on every half
//! cycle), with a stack of harmonics and a papery buzz on top.
//!
//! Milestone 2 plays one hum whose loudness follows how many lit tubes are
//! near the player. Placing a separate source at each fixture comes with the
//! audio milestone.

use std::f32::consts::TAU;
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, SampleRate, Source, Volume};
use bevy::prelude::*;

use crate::lights::FixtureLight;
use crate::player::Player;

const SAMPLE_RATE: u32 = 44_100;
/// Mains frequency in hertz. The hum sits at twice this.
const MAINS: f32 = 60.0;
/// Loudness with a lit panel directly overhead.
const FULL: f32 = 0.5;

/// The sound itself is the same everywhere, so the asset carries no data.
#[derive(Asset, TypePath)]
pub struct Hum;

/// Produces the samples. Bevy pulls from this iterator on the audio thread.
pub struct HumDecoder {
    /// Sample number within the current second. Every component is a whole
    /// number of hertz, so the wave repeats exactly each second, and counting
    /// in integers means it never drifts however long the game runs.
    sample: u32,
    /// State of the noise generator and of the filter that shapes it.
    noise: u32,
    smoothed: f32,
}

impl Iterator for HumDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let t = self.sample as f32 / SAMPLE_RATE as f32;
        self.sample = (self.sample + 1) % SAMPLE_RATE;
        let wave = |harmonic: f32, phase: f32| (TAU * MAINS * harmonic * t + phase).sin();

        // The hum: strong second harmonic of the mains, weaker ones above.
        let hum = 0.50 * wave(2.0, 0.0)
            + 0.22 * wave(4.0, 0.9)
            + 0.12 * wave(6.0, 2.1)
            + 0.07 * wave(8.0, 0.4)
            + 0.05 * wave(1.0, 1.3)
            + 0.03 * wave(12.0, 2.7);

        // The buzz: hiss that swells on every half cycle of the mains.
        // Xorshift is a tiny random generator; this noise is only texture
        // for the ear and has nothing to do with the world generator.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        let white = self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0;
        self.smoothed += 0.35 * (white - self.smoothed);
        let buzz = self.smoothed * (0.5 + 0.5 * wave(2.0, 0.6)).powi(3);

        Some(0.6 * hum + 0.25 * buzz)
    }
}

impl Source for HumDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(1).expect("one channel is valid")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).expect("sample rate is non-zero")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for Hum {
    type Decoder = HumDecoder;

    fn decoder(&self) -> HumDecoder {
        HumDecoder { sample: 0, noise: 0x2545_F491, smoothed: 0.0 }
    }
}

#[derive(Component)]
struct HumPlayer;

pub struct HumPlugin;

impl Plugin for HumPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Hum>().add_systems(Startup, start).add_systems(Update, follow_the_lights);
    }
}

fn start(mut commands: Commands, mut hums: ResMut<Assets<Hum>>) {
    commands.spawn((HumPlayer, AudioPlayer(hums.add(Hum)), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))));
}

/// How loud the hum is for a listener, given each nearby fixture's distance
/// in metres and its current output (0.0 to 1.0).
pub fn loudness(fixtures: impl Iterator<Item = (f32, f32)>) -> f32 {
    // Each tube contributes less with distance; many tubes add up, but with
    // diminishing returns, as loudness does.
    let sum: f32 = fixtures.map(|(distance, level)| level / (1.0 + distance * distance * 0.5)).sum();
    FULL * (sum / (sum + 1.0)) * 1.6
}

fn follow_the_lights(
    time: Res<Time>,
    player: Single<&Transform, With<Player>>,
    fixtures: Query<(&GlobalTransform, &FixtureLight)>,
    mut sink: Single<&mut AudioSink, With<HumPlayer>>,
    mut current: Local<f32>,
) {
    let ear = player.translation;
    let nearby = fixtures
        .iter()
        .map(|(at, fixture)| (at.translation().distance(ear), fixture.level))
        .filter(|&(distance, level)| distance < 14.0 && level > 0.0);
    let target = loudness(nearby);
    // Glide rather than jump, except that a flickering tube overhead should
    // be heard to stutter: a short time constant does both.
    *current += (target - *current) * (1.0 - (-18.0 * time.delta_secs()).exp());
    sink.set_volume(Volume::Linear(*current));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_stay_within_range_and_are_not_silent() {
        let mut peak = 0.0f32;
        for sample in Hum.decoder().take(SAMPLE_RATE as usize * 2) {
            assert!(sample.is_finite());
            peak = peak.max(sample.abs());
        }
        assert!(peak > 0.2 && peak <= 1.0, "peak {peak}");
    }

    #[test]
    fn the_wave_repeats_each_second_without_a_click() {
        let samples: Vec<f32> = Hum.decoder().take(SAMPLE_RATE as usize + 2).collect();
        // The step across the one-second wrap is no bigger than any other.
        let biggest = samples.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        let at_wrap = (samples[SAMPLE_RATE as usize] - samples[SAMPLE_RATE as usize - 1]).abs();
        assert!(at_wrap <= biggest && biggest < 0.2, "wrap step {at_wrap}, biggest {biggest}");
    }

    #[test]
    fn darkness_is_silent_and_light_is_not() {
        assert_eq!(loudness(std::iter::empty()), 0.0);
        let one = loudness([(1.0, 1.0)].into_iter());
        let many = loudness((0..12).map(|i| (1.0 + i as f32, 1.0)));
        assert!(one > 0.1 && many > one && many <= FULL * 1.6);
        assert!(loudness([(1.0, 0.25)].into_iter()) < one);
    }
}
