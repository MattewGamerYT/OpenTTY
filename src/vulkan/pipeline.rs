use std::ffi::CStr;
use std::io::Cursor;

use ash::vk;

const VERT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/glyph.vert.spv"));
const FRAG_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/glyph.frag.spv"));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CellInstance {
    pub cell_rect: [f32; 4],
    pub glyph_rect: [f32; 4],
    pub uv_rect: [f32; 4],
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub flags: u32,
    pub cursor_rgba: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PushConstants {
    pub viewport_size: [f32; 2],
    pub is_srgb: u32,
    pub _pad: u32,
}

pub const FLAG_UNDERLINE: u32 = 1 << 0;
pub const FLAG_STRIKETHROUGH: u32 = 1 << 1;
pub const FLAG_CURSOR_BLOCK: u32 = 1 << 2;
pub const FLAG_CURSOR_BEAM: u32 = 1 << 3;
pub const FLAG_CURSOR_UNDERLINE: u32 = 1 << 4;

pub struct GlyphPipeline {
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_set: vk::DescriptorSet,
    pub pipeline_layout: vk::PipelineLayout,
    pub pipeline: vk::Pipeline,
}

impl GlyphPipeline {
    pub fn create(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        atlas_view: vk::ImageView,
        atlas_sampler: vk::Sampler,
    ) -> Result<Self, String> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];

        let dsl_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let descriptor_set_layout = unsafe { device.create_descriptor_set_layout(&dsl_info, None) }
            .map_err(|e| format!("create_descriptor_set_layout failed: {e}"))?;

        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: 1,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: 1,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(1);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("create_descriptor_pool failed: {e}"))?;

        let set_layouts = [descriptor_set_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&set_layouts);
        let descriptor_set = unsafe { device.allocate_descriptor_sets(&alloc_info) }
            .map_err(|e| format!("allocate_descriptor_sets failed: {e}"))?[0];

        let image_info = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(atlas_view);
        let sampler_info = vk::DescriptorImageInfo::default().sampler(atlas_sampler);

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(std::slice::from_ref(&image_info)),
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(std::slice::from_ref(&sampler_info)),
        ];
        unsafe { device.update_descriptor_sets(&writes, &[]) };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .offset(0)
            .size(std::mem::size_of::<PushConstants>() as u32);

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(std::slice::from_ref(&push_range));
        let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
            .map_err(|e| format!("create_pipeline_layout failed: {e}"))?;

        let vert_module = create_shader_module(device, VERT_SPV)?;
        let frag_module = create_shader_module(device, FRAG_SPV)?;

        let vert_entry = CStr::from_bytes_with_nul(b"vs_main\0").unwrap();
        let frag_entry = CStr::from_bytes_with_nul(b"fs_main\0").unwrap();

        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(vert_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(frag_entry),
        ];

        let binding_desc = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<CellInstance>() as u32)
            .input_rate(vk::VertexInputRate::INSTANCE);

        let attr_descs = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 16,
            },
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: vk::Format::R32G32B32A32_SFLOAT,
                offset: 32,
            },
            vk::VertexInputAttributeDescription {
                location: 3,
                binding: 0,
                format: vk::Format::R32_UINT,
                offset: 48,
            },
            vk::VertexInputAttributeDescription {
                location: 4,
                binding: 0,
                format: vk::Format::R32_UINT,
                offset: 52,
            },
            vk::VertexInputAttributeDescription {
                location: 5,
                binding: 0,
                format: vk::Format::R32_UINT,
                offset: 56,
            },
            vk::VertexInputAttributeDescription {
                location: 6,
                binding: 0,
                format: vk::Format::R32_UINT,
                offset: 60,
            },
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(std::slice::from_ref(&binding_desc))
            .vertex_attribute_descriptions(&attr_descs);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&color_blend_attachment));

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);

        let pipeline = unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }
        .map_err(|(_, e)| format!("create_graphics_pipelines failed: {e}"))?[0];

        unsafe {
            device.destroy_shader_module(vert_module, None);
            device.destroy_shader_module(frag_module, None);
        }

        Ok(Self {
            descriptor_set_layout,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            pipeline,
        })
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_pool(self.descriptor_pool, None);
            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

fn create_shader_module(device: &ash::Device, bytes: &[u8]) -> Result<vk::ShaderModule, String> {
    let mut cursor = Cursor::new(bytes);
    let words = ash::util::read_spv(&mut cursor)
        .map_err(|e| format!("read_spv failed: {e}"))?;
    let info = vk::ShaderModuleCreateInfo::default().code(&words);
    unsafe { device.create_shader_module(&info, None) }
        .map_err(|e| format!("create_shader_module failed: {e}"))
}
