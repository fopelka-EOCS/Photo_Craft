// A layer's moving pixels drawn through a live transform's mesh (`photocraft_compose::live`):
// the content of the moving layer while a Free Transform, Warp, Puppet Warp or Perspective Warp
// box is open. The rest of the stack composites it like any layer content. Mirrors
// `photocraft_compose::live::rasterize`: bilinear over premultiplied texels, transparent past
// the source, straight alpha out; a later triangle replaces an earlier one (no blending).

struct Chunk {
    origin: vec2<i32>,
    size: vec2<i32>,
};

struct Live {
    // Side of the chunk textures the mesh is drawn into (pixels).
    chunk_side: vec2<f32>,
    // Source pixel (document coordinates) at the source texture's top-left texel, and the
    // texture's size in texels.
    src_origin: vec2<f32>,
    src_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> chunk: Chunk;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_sampler: sampler;
@group(1) @binding(2) var<uniform> live: Live;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_mesh(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VOut {
    // Document pixels to the chunk texture's clip space (its texel (0, 0) is `chunk.origin`).
    let p = (pos - vec2<f32>(chunk.origin)) / live.chunk_side;
    var o: VOut;
    o.pos = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    o.uv = (uv - live.src_origin) / live.src_size;
    return o;
}

@fragment
fn fs_mesh(in: VOut) -> @location(0) vec4<f32> {
    // The texture holds premultiplied texels with a transparent border, sampled clamped to it.
    let c = textureSampleLevel(src_tex, src_sampler, in.uv, 0.0);
    if (c.a <= 1e-6) {
        return vec4(0.0);
    }
    return vec4(c.rgb / c.a, min(c.a, 1.0));
}
