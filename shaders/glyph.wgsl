struct PushConstants {
    viewport_size: vec2<f32>,
}

var<push_constant> pc: PushConstants;

@group(0) @binding(0)
var atlas_tex: texture_2d<f32>;

@group(0) @binding(1)
var atlas_sampler: sampler;

struct InstanceInput {
    @location(0) cell_rect: vec4<f32>,
    @location(1) glyph_rect: vec4<f32>,
    @location(2) uv_rect: vec4<f32>,
    @location(3) fg_rgba: u32,
    @location(4) bg_rgba: u32,
    @location(5) flags: u32,
    @location(6) cursor_rgba: u32,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_px: vec2<f32>,
    @location(1) cell_size: vec2<f32>,
    @location(2) glyph_rect: vec4<f32>,
    @location(3) uv_rect: vec4<f32>,
    @location(4) fg_color: vec4<f32>,
    @location(5) bg_color: vec4<f32>,
    @location(6) cursor_color: vec4<f32>,
    @location(7) @interpolate(flat) flags: u32,
}

fn unpack_rgba(packed: u32) -> vec4<f32> {
    let r = f32((packed >> 24u) & 0xFFu) / 255.0;
    let g = f32((packed >> 16u) & 0xFFu) / 255.0;
    let b = f32((packed >> 8u) & 0xFFu) / 255.0;
    let a = f32(packed & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    inst: InstanceInput,
) -> VertexOutput {
    var quad = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = quad[vertex_index];
    let cell_origin = inst.cell_rect.xy;
    let cell_size = inst.cell_rect.zw;
    let px = cell_origin + corner * cell_size;

    // Vulkan NDC: (-1, -1) top-left, (+1, +1) bottom-right
    let ndc = (px / max(pc.viewport_size, vec2<f32>(1.0, 1.0))) * 2.0 - 1.0;

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
    out.local_px = corner * cell_size;
    out.cell_size = cell_size;
    out.glyph_rect = inst.glyph_rect;
    out.uv_rect = inst.uv_rect;
    out.fg_color = unpack_rgba(inst.fg_rgba);
    out.bg_color = unpack_rgba(inst.bg_rgba);
    out.cursor_color = unpack_rgba(inst.cursor_rgba);
    out.flags = inst.flags;
    return out;
}

// Instance flags
const FLAG_UNDERLINE: u32 = 1u << 0u;
const FLAG_STRIKETHROUGH: u32 = 1u << 1u;
const FLAG_CURSOR_BLOCK: u32 = 1u << 2u;
const FLAG_CURSOR_BEAM: u32 = 1u << 3u;
const FLAG_CURSOR_UNDERLINE: u32 = 1u << 4u;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.bg_color;
    var fg = in.fg_color;

    // Block cursor swaps cell background and foreground cleanly
    if (in.flags & FLAG_CURSOR_BLOCK) != 0u {
        fg = color;
        color = in.cursor_color;
    }

    // Sample glyph atlas if fragment falls within glyph bitmap bounds
    let g_min = in.glyph_rect.xy;
    let g_size = in.glyph_rect.zw;
    let g_max = g_min + g_size;

    if g_size.x > 0.0 && g_size.y > 0.0
        && in.local_px.x >= g_min.x && in.local_px.x < g_max.x
        && in.local_px.y >= g_min.y && in.local_px.y < g_max.y
    {
        let rel = (in.local_px - g_min) / g_size;
        let uv = mix(in.uv_rect.xy, in.uv_rect.zw, rel);
        let coverage = textureSampleLevel(atlas_tex, atlas_sampler, uv, 0.0).r;
        color = mix(color, fg, coverage * fg.a);
    }

    // Underline
    if (in.flags & FLAG_UNDERLINE) != 0u {
        let u_y = max(in.cell_size.y - 2.0, 0.0);
        if in.local_px.y >= u_y && in.local_px.y < (u_y + 1.5) {
            color = fg;
        }
    }

    // Strikethrough
    if (in.flags & FLAG_STRIKETHROUGH) != 0u {
        let s_y = in.cell_size.y * 0.5;
        if in.local_px.y >= (s_y - 0.75) && in.local_px.y < (s_y + 0.75) {
            color = fg;
        }
    }

    // Beam cursor
    if (in.flags & FLAG_CURSOR_BEAM) != 0u {
        if in.local_px.x < 2.0 {
            color = in.cursor_color;
        }
    }

    // Underline cursor
    if (in.flags & FLAG_CURSOR_UNDERLINE) != 0u {
        let c_y = max(in.cell_size.y - 2.0, 0.0);
        if in.local_px.y >= c_y {
            color = in.cursor_color;
        }
    }

    return vec4<f32>(color.rgb, 1.0);
}
