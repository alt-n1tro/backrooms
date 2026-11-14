// Procedural surfaces of the yellow rooms: wallpaper, carpet, ceiling tiles.
//
// There are no textures. Every pattern and stain is computed from where the
// pixel is in the world, so each wall is unique and comes out the same every
// time. Positions reach the shader relative to the render origin; adding the
// origin's cell (an integer) gives the true cell, and all hashing is done on
// those integers so it stays exact however far from 0,0 the player is.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
};

struct Surface {
    // Cell at world position zero: x east, y north.
    origin: vec2<i32>,
    // Ceiling height in metres.
    ceiling: f32,
    pad: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> surface: Surface;

// Integer hash of a lattice point to 0..1.
fn hash(p: vec2<i32>, salt: u32) -> f32 {
    var h = bitcast<u32>(p.x) * 0x9E3779B1u;
    h = h ^ (bitcast<u32>(p.y) * 0x85EBCA77u);
    h = h ^ (salt * 0xC2B2AE3Du);
    h = h ^ (h >> 15u);
    h = h * 0x2C1B3C6Du;
    h = h ^ (h >> 12u);
    h = h * 0x297A2D39u;
    h = h ^ (h >> 15u);
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Smooth value noise with `per_cell` lattice points per cell (0.5 m).
// `cell` is the integer cell and `f` the position inside it, 0..1.
fn noise(cell: vec2<i32>, f: vec2<f32>, per_cell: i32, salt: u32) -> f32 {
    let g = f * f32(per_cell);
    let gi = floor(g);
    let t = g - gi;
    let base = cell * per_cell + vec2<i32>(gi);
    let s = t * t * (3.0 - 2.0 * t);
    let a = hash(base, salt);
    let b = hash(base + vec2<i32>(1, 0), salt);
    let c = hash(base + vec2<i32>(0, 1), salt);
    let d = hash(base + vec2<i32>(1, 1), salt);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// Three octaves of noise, largest feature one cell across.
fn cloud(cell: vec2<i32>, f: vec2<f32>, salt: u32) -> f32 {
    return noise(cell, f, 1, salt) * 0.55 + noise(cell, f, 2, salt + 1u) * 0.3 + noise(cell, f, 4, salt + 2u) * 0.15;
}

// How much of a detail with `per_cell` features per cell survives at this
// distance. Detail smaller than a pixel is faded out rather than left to
// shimmer.
fn keep(cells_per_pixel: f32, per_cell: f32) -> f32 {
    return clamp(1.5 - cells_per_pixel * per_cell * 1.5, 0.0, 1.0);
}

struct Look {
    color: vec3<f32>,
    roughness: f32,
}

fn carpet(cell: vec2<i32>, f: vec2<f32>, damp_in: f32, wear_in: f32, cpp: f32) -> Look {
    // Pile: fine fibre noise over a slightly blotchy base.
    let fibre = (noise(cell, f, 64, 11u) - 0.5) * keep(cpp, 64.0) + (noise(cell, f, 16, 12u) - 0.5) * 0.6 * keep(cpp, 16.0);
    let blotch = cloud(cell, f, 13u);
    // Mottling a few centimetres across: what a carpet looks like once the
    // individual fibres are too small to see.
    let mottle = (noise(cell, f, 8, 14u) - 0.5) * keep(cpp, 8.0);
    var color = vec3<f32>(0.17, 0.115, 0.04) * (0.84 + 0.3 * blotch + 0.22 * fibre + 0.2 * mottle);

    // Worn paths: flattened, paler pile.
    let worn = smoothstep(0.35, 0.9, wear_in + (cloud(cell, f, 20u) - 0.5) * 0.5);
    color = mix(color, vec3<f32>(0.21, 0.165, 0.085) * (0.9 + 0.1 * fibre), worn * 0.55);

    // Damp: ragged-edged dark patches that catch the light.
    let wet = smoothstep(0.12, 0.5, damp_in + (cloud(cell, f, 30u) - 0.5) * 0.35);
    color = color * mix(1.0, 0.4, wet);
    return Look(color, mix(0.97, 0.72, wet));
}

fn tiles(cell: vec2<i32>, f: vec2<f32>, damp_in: f32, wear_in: f32, cpp: f32) -> Look {
    // One tile per cell, with a shadowed gap around it.
    let edge = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
    let gap = 1.0 - smoothstep(0.012, 0.03, edge);
    // Each tile is a slightly different white, with pinhole speckle.
    let tile = hash(cell, 40u);
    let speckle = step(0.93, noise(cell, f, 48, 41u)) * keep(cpp, 48.0);
    var color = vec3<f32>(0.62, 0.60, 0.52) * (0.92 + 0.1 * tile) * (1.0 - 0.35 * speckle);

    // Water stains: brown blooms with a darker rim.
    let stain = damp_in * 0.9 + wear_in * 0.35 + (cloud(cell, f, 42u) - 0.5) * 0.5;
    let bloom = smoothstep(0.3, 0.55, stain);
    let rim = smoothstep(0.3, 0.38, stain) * (1.0 - smoothstep(0.38, 0.5, stain));
    color = mix(color, vec3<f32>(0.36, 0.25, 0.10), bloom * 0.6 + rim * 0.3);
    color = color * (1.0 - 0.75 * gap * keep(cpp, 12.0));
    return Look(color, 0.92);
}

// `along` is the cell index and fraction along the wall, `across` the cell
// index through it (which side of which wall this is), `height` in metres.
fn wallpaper(along_cell: i32, along_f: f32, across: i32, height: f32, damp_in: f32, wear_in: f32, cpp: f32) -> Look {
    let salt = bitcast<u32>(across) * 0x27D4EB2Fu;
    // A lattice on the wall face: half-metre squares.
    let v = height * 2.0;
    let wall_cell = vec2<i32>(along_cell, i32(floor(v)));
    let wall_f = vec2<f32>(along_f, v - floor(v));

    // The pattern: bands of chevrons, two per cell, in two close yellows,
    // with a pinstripe where the chevrons meet.
    let u = along_f * 2.0;
    let zig = abs(u - floor(u) - 0.5) * 2.0;
    let band_pos = height * 7.0 + zig * 0.5;
    let band = band_pos - floor(band_pos);
    let chevron = smoothstep(0.42, 0.5, band) * (1.0 - smoothstep(0.92, 1.0, band));
    let stripe = 1.0 - smoothstep(0.0, 0.04, min(zig, 1.0 - zig));
    let pattern = (chevron * 0.75 + stripe * 0.4) * keep(cpp, 14.0);

    let light = vec3<f32>(0.58, 0.40, 0.085);
    let dark = vec3<f32>(0.47, 0.31, 0.055);
    let paper_grain = (noise(wall_cell, wall_f, 32, salt + 1u) - 0.5) * 0.1 * keep(cpp, 32.0);
    var color = mix(light, dark, clamp(pattern, 0.0, 1.0)) * (1.0 + paper_grain);

    // Age: uneven yellowing and grime, heavier towards the floor.
    let grime = cloud(wall_cell, wall_f, salt + 2u);
    let low = 1.0 - smoothstep(0.0, 1.2, height);
    color = color * (1.0 - (0.12 + 0.45 * wear_in) * grime * (0.5 + 0.5 * low));

    // Damp creeping up from the carpet, with a ragged tide line.
    let tide = damp_in * 1.6 + (cloud(wall_cell, wall_f, salt + 3u) - 0.5) * 0.6 - height * 0.55;
    let wet = smoothstep(0.0, 0.25, tide);
    color = color * mix(1.0, 0.55, wet);

    // Where wear is heavy the paper has come away from the plaster.
    let peel = smoothstep(0.68, 0.74, wear_in * 0.45 + cloud(wall_cell, wall_f, salt + 4u) * 0.5);
    color = mix(color, vec3<f32>(0.42, 0.39, 0.31) * (0.9 + 0.2 * grime), peel);

    // Skirting board.
    let skirting = 1.0 - smoothstep(0.085, 0.09, height);
    color = mix(color, vec3<f32>(0.20, 0.13, 0.03) * (0.8 + 0.4 * grime), skirting);
    return Look(color, mix(0.82, 0.55, max(wet, skirting)));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Position on the floor plan in cells: x east, y north (north is -Z).
    let plan = vec2<f32>(in.world_position.x, -in.world_position.z) * 2.0;
    let whole = floor(plan);
    let cell = vec2<i32>(whole) + surface.origin;
    let f = plan - whole;
    let height = in.world_position.y;
    let cells_per_pixel = max(length(fwidth(plan)), fwidth(height) * 2.0);

    // The mesher stores moisture in red, wear in green and baked light (in
    // thousands of lux) in blue.
    let damp = in.color.r;
    let wear = in.color.g;
    let lux = in.color.b * 1000.0;

    let n = in.world_normal;
    var look: Look;
    if n.y > 0.5 {
        look = carpet(cell, f, damp, wear, cells_per_pixel);
    } else if n.y < -0.5 {
        look = tiles(cell, f, damp, wear, cells_per_pixel);
    } else if abs(n.x) > 0.5 {
        look = wallpaper(cell.y, f.y, cell.x, height, damp, wear, cells_per_pixel);
    } else {
        look = wallpaper(cell.x, f.x, cell.y + 0x4000, height, damp, wear, cells_per_pixel);
    }

    pbr_input.material.base_color = vec4<f32>(look.color, 1.0);
    pbr_input.material.perceptual_roughness = look.roughness;

    // Light from the steady panels was worked out when the chunk was built.
    // A matt surface receiving E lux sends back colour * E / pi, tinted by
    // the tube. It is passed on as "emissive" so that Bevy adds it to
    // whatever the live lights (flickering tubes) contribute; the 1.0 in the
    // last place makes camera exposure apply to it like any other light.
    let tube = vec3<f32>(1.0, 0.93, 0.71);
    pbr_input.material.emissive = vec4<f32>(look.color * tube * lux / 3.14159265, 1.0);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
