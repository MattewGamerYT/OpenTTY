use std::env;
use std::fs;
use std::path::PathBuf;

use naga::ShaderStage;
use naga::back::spv::{self, Options as SpvOptions, PipelineOptions};
use naga::front::wgsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};

fn compile_stage(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: ShaderStage,
    entry_point: &str,
    out_path: &PathBuf,
) {
    let spv_options = SpvOptions {
        lang_version: (1, 3),
        ..Default::default()
    };
    let pipeline_options = PipelineOptions {
        shader_stage: stage,
        entry_point: entry_point.to_string(),
    };
    let words = spv::write_vec(module, info, &spv_options, Some(&pipeline_options))
        .unwrap_or_else(|err| panic!("failed to emit SPIR-V for {entry_point}: {err:?}"));

    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    fs::write(out_path, bytes)
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", out_path.display()));
}

fn main() {
    println!("cargo:rerun-if-changed=shaders/glyph.wgsl");

    let shader_src = fs::read_to_string("shaders/glyph.wgsl")
        .expect("failed to read shaders/glyph.wgsl");

    let module = wgsl::parse_str(&shader_src)
        .unwrap_or_else(|err| panic!("WGSL parse error:\n{}", err.emit_to_string(&shader_src)));

    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .expect("WGSL validation failed");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));

    compile_stage(
        &module,
        &info,
        ShaderStage::Vertex,
        "vs_main",
        &out_dir.join("glyph.vert.spv"),
    );
    compile_stage(
        &module,
        &info,
        ShaderStage::Fragment,
        "fs_main",
        &out_dir.join("glyph.frag.spv"),
    );
}
