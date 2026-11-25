pub mod atlas;
pub mod pipeline;
pub mod swapchain;

use std::ffi::{CStr, c_void};

use ash::vk;

use crate::config::Config;
use crate::font::{FontEngine, GlyphKey};
use crate::terminal::Terminal;
use crate::terminal::color::resolve_rgba;
use crate::terminal::grid::CursorStyle;

use self::atlas::{GlyphAtlas, find_memory_type};
use self::pipeline::{
    CellInstance, FLAG_CURSOR_BEAM, FLAG_CURSOR_BLOCK, FLAG_CURSOR_UNDERLINE, FLAG_STRIKETHROUGH,
    FLAG_UNDERLINE, GlyphPipeline, PushConstants,
};
use self::swapchain::{SwapchainState, create_render_pass};

const MAX_FRAMES_IN_FLIGHT: usize = 2;

struct FrameSync {
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
    command_buffer: vk::CommandBuffer,
    instance_buffer: vk::Buffer,
    instance_memory: vk::DeviceMemory,
    instance_ptr: *mut CellInstance,
    instance_capacity: usize,
}

pub struct VulkanRenderer {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    device: ash::Device,
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,
    render_pass: vk::RenderPass,
    swapchain: SwapchainState,
    atlas: GlyphAtlas,
    pipeline: GlyphPipeline,
    frames: Vec<FrameSync>,
    current_frame: usize,
    instances_scratch: Vec<CellInstance>,
}

impl VulkanRenderer {
    pub fn new(
        wl_display: *mut c_void,
        wl_surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| format!("failed to load Vulkan ICD loader: {e}"))?;

        let app_name = CStr::from_bytes_with_nul(b"OpenTTY\0").unwrap();
        let engine_name = CStr::from_bytes_with_nul(b"OpenTTY\0").unwrap();
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_2);

        // Strictly native Wayland Vulkan surface extensions: zero X11, zero OpenGL.
        let instance_exts = [
            ash::khr::surface::NAME.as_ptr(),
            ash::khr::wayland_surface::NAME.as_ptr(),
        ];

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&instance_exts);

        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|e| format!("vkCreateInstance failed: {e}"))?;

        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let wayland_surface_loader = ash::khr::wayland_surface::Instance::new(&entry, &instance);

        let surface_info = vk::WaylandSurfaceCreateInfoKHR::default()
            .display(wl_display)
            .surface(wl_surface);
        let surface = unsafe { wayland_surface_loader.create_wayland_surface(&surface_info, None) }
            .map_err(|e| format!("vkCreateWaylandSurfaceKHR failed: {e}"))?;

        let (physical_device, queue_family_index) =
            pick_physical_device(&instance, &surface_loader, surface)?;
        let mem_props = unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let queue_priority = [1.0_f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priority);

        let device_exts = [ash::khr::swapchain::NAME.as_ptr()];
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_exts);

        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|e| format!("vkCreateDevice failed: {e}"))?;
        let graphics_queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { device.create_command_pool(&pool_info, None) }
            .map_err(|e| format!("create_command_pool failed: {e}"))?;

        let formats = unsafe {
            surface_loader.get_physical_device_surface_formats(physical_device, surface)
        }
        .map_err(|e| format!("get_physical_device_surface_formats failed: {e}"))?;
        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or_else(|| formats.iter().find(|f| f.format == vk::Format::R8G8B8A8_UNORM))
            .map(|f| f.format)
            .unwrap_or(formats[0].format);

        let render_pass = create_render_pass(&device, format)?;
        let swapchain = SwapchainState::create(
            &instance,
            &device,
            physical_device,
            &surface_loader,
            surface,
            render_pass,
            width,
            height,
            vk::SwapchainKHR::null(),
        )?;

        let atlas = GlyphAtlas::new(&device, &mem_props)?;
        let pipeline = GlyphPipeline::create(&device, render_pass, atlas.view, atlas.sampler)?;

        let cmd_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(MAX_FRAMES_IN_FLIGHT as u32);
        let cmd_buffers = unsafe { device.allocate_command_buffers(&cmd_alloc) }
            .map_err(|e| format!("allocate_command_buffers failed: {e}"))?;

        let mut frames = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let initial_instance_cap = 32_768usize;
        for cmd in cmd_buffers {
            let sem_info = vk::SemaphoreCreateInfo::default();
            let fence_info =
                vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
            let image_available = unsafe { device.create_semaphore(&sem_info, None) }
                .map_err(|e| format!("create_semaphore failed: {e}"))?;
            let render_finished = unsafe { device.create_semaphore(&sem_info, None) }
                .map_err(|e| format!("create_semaphore failed: {e}"))?;
            let in_flight = unsafe { device.create_fence(&fence_info, None) }
                .map_err(|e| format!("create_fence failed: {e}"))?;

            let (instance_buffer, instance_memory, instance_ptr) =
                create_instance_buffer(&device, &mem_props, initial_instance_cap)?;

            frames.push(FrameSync {
                image_available,
                render_finished,
                in_flight,
                command_buffer: cmd,
                instance_buffer,
                instance_memory,
                instance_ptr,
                instance_capacity: initial_instance_cap,
            });
        }

        Ok(Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            mem_props,
            device,
            graphics_queue,
            command_pool,
            render_pass,
            swapchain,
            atlas,
            pipeline,
            frames,
            current_frame: 0,
            instances_scratch: Vec::with_capacity(8192),
        })
    }

    pub fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<(), String> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| format!("device_wait_idle failed: {e}"))?;
            self.swapchain.destroy_views_and_framebuffers(&self.device);
        }

        let old = self.swapchain.swapchain;
        self.swapchain = SwapchainState::create(
            &self.instance,
            &self.device,
            self.physical_device,
            &self.surface_loader,
            self.surface,
            self.render_pass,
            width.max(1),
            height.max(1),
            old,
        )?;
        Ok(())
    }

    pub fn clear_atlas(&mut self) {
        self.atlas.clear();
    }

    pub fn render_and_present(
        &mut self,
        terminal: &Terminal,
        font: &mut FontEngine,
        config: &Config,
        scale_factor: f32,
    ) -> Result<bool, String> {
        let frame_idx = self.current_frame;
        let in_flight = self.frames[frame_idx].in_flight;
        let image_available = self.frames[frame_idx].image_available;
        let render_finished = self.frames[frame_idx].render_finished;
        let cmd = self.frames[frame_idx].command_buffer;

        unsafe {
            self.device
                .wait_for_fences(&[in_flight], true, u64::MAX)
                .map_err(|e| format!("wait_for_fences failed: {e}"))?;
        }

        let acquire = unsafe {
            self.swapchain.loader.acquire_next_image(
                self.swapchain.swapchain,
                u64::MAX,
                image_available,
                vk::Fence::null(),
            )
        };

        let image_index = match acquire {
            Ok((idx, _suboptimal)) => idx,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(true),
            Err(e) => return Err(format!("acquire_next_image failed: {e}")),
        };

        // Build per-cell instances on CPU and copy to persistent mapped buffer.
        self.build_cell_instances(terminal, font, config, scale_factor);
        let instance_count = self.instances_scratch.len();

        if instance_count > self.frames[frame_idx].instance_capacity {
            let new_cap = instance_count.next_power_of_two();
            unsafe {
                self.device.unmap_memory(self.frames[frame_idx].instance_memory);
                self.device
                    .destroy_buffer(self.frames[frame_idx].instance_buffer, None);
                self.device
                    .free_memory(self.frames[frame_idx].instance_memory, None);
            }
            let (buf, mem, ptr) =
                create_instance_buffer(&self.device, &self.mem_props, new_cap)?;
            self.frames[frame_idx].instance_buffer = buf;
            self.frames[frame_idx].instance_memory = mem;
            self.frames[frame_idx].instance_ptr = ptr;
            self.frames[frame_idx].instance_capacity = new_cap;
        }

        if instance_count > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.instances_scratch.as_ptr(),
                    self.frames[frame_idx].instance_ptr,
                    instance_count,
                );
            }
        }

        unsafe {
            self.device
                .reset_fences(&[in_flight])
                .map_err(|e| format!("reset_fences failed: {e}"))?;
            self.device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                .map_err(|e| format!("reset_command_buffer failed: {e}"))?;

            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(cmd, &begin_info)
                .map_err(|e| format!("begin_command_buffer failed: {e}"))?;
        }

        self.atlas.record_uploads(&self.device, cmd);

        let extent = self.swapchain.extent;
        let is_srgb = is_srgb_format(self.swapchain.format);
        let bg = config.default_bg;
        let bg_r = ((bg >> 24) & 0xFF) as f32 / 255.0;
        let bg_g = ((bg >> 16) & 0xFF) as f32 / 255.0;
        let bg_b = ((bg >> 8) & 0xFF) as f32 / 255.0;
        let clear_rgb = if is_srgb {
            [
                srgb_to_linear(bg_r),
                srgb_to_linear(bg_g),
                srgb_to_linear(bg_b),
                1.0,
            ]
        } else {
            [bg_r, bg_g, bg_b, 1.0]
        };
        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue { float32: clear_rgb },
        };

        let rp_begin = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.swapchain.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(std::slice::from_ref(&clear_color));

        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        };
        let push = PushConstants {
            viewport_size: [extent.width as f32, extent.height as f32],
            is_srgb: u32::from(is_srgb),
            _pad: 0,
        };
        let push_bytes = unsafe {
            std::slice::from_raw_parts(
                (&push as *const PushConstants) as *const u8,
                std::mem::size_of::<PushConstants>(),
            )
        };

        unsafe {
            self.device
                .cmd_begin_render_pass(cmd, &rp_begin, vk::SubpassContents::INLINE);
            self.device
                .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline.pipeline);
            self.device.cmd_set_viewport(cmd, 0, &[viewport]);
            self.device.cmd_set_scissor(cmd, 0, &[scissor]);
            self.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline.pipeline_layout,
                0,
                &[self.pipeline.descriptor_set],
                &[],
            );
            self.device.cmd_push_constants(
                cmd,
                self.pipeline.pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                push_bytes,
            );

            // Single-draw-call instanced glyph & cell rendering!
            if instance_count > 0 {
                let instance_buf = self.frames[frame_idx].instance_buffer;
                self.device.cmd_bind_vertex_buffers(cmd, 0, &[instance_buf], &[0]);
                self.device.cmd_draw(cmd, 6, instance_count as u32, 0, 0);
            }

            self.device.cmd_end_render_pass(cmd);
            self.device
                .end_command_buffer(cmd)
                .map_err(|e| format!("end_command_buffer failed: {e}"))?;
        }

        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(std::slice::from_ref(&image_available))
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(std::slice::from_ref(&cmd))
            .signal_semaphores(std::slice::from_ref(&render_finished));

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], in_flight)
                .map_err(|e| format!("queue_submit failed: {e}"))?;
        }

        let swapchains = [self.swapchain.swapchain];
        let indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(std::slice::from_ref(&render_finished))
            .swapchains(&swapchains)
            .image_indices(&indices);

        let present_res = unsafe {
            self.swapchain
                .loader
                .queue_present(self.graphics_queue, &present_info)
        };

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;

        match present_res {
            Ok(suboptimal) => Ok(suboptimal),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(true),
            Err(e) => Err(format!("queue_present failed: {e}")),
        }
    }

    fn build_cell_instances(
        &mut self,
        terminal: &Terminal,
        font: &mut FontEngine,
        config: &Config,
        scale_factor: f32,
    ) {
        self.instances_scratch.clear();
        let grid = terminal.active_grid();
        let metrics = font.metrics();
        let pad = (config.padding_px * scale_factor).round();
        let cw = metrics.cell_width;
        let ch = metrics.cell_height;
        let baseline = metrics.baseline;

        let cursor_visible = grid.cursor.visible && grid.scroll_offset == 0;
        let cursor_row = grid.cursor.row;
        let cursor_col = grid.cursor.col;

        for row_idx in 0..grid.rows {
            let Some(row) = terminal.visible_row(row_idx) else {
                continue;
            };
            let cell_y = pad + (row_idx as f32) * ch;

            for col_idx in 0..grid.cols.min(row.cells.len()) {
                let cell = row.cells[col_idx];
                let cell_x = pad + (col_idx as f32) * cw;

                let mut fg = resolve_rgba(cell.fg, config.default_fg, config.default_bg);
                let mut bg = resolve_rgba(cell.bg, config.default_fg, config.default_bg);
                if cell.flags.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.flags.dim {
                    fg = dim_rgba(fg);
                }
                if cell.flags.hidden {
                    fg = bg;
                }

                let mut flags = 0u32;
                if cell.flags.underline {
                    flags |= FLAG_UNDERLINE;
                }
                if cell.flags.strikethrough {
                    flags |= FLAG_STRIKETHROUGH;
                }

                let is_cursor = cursor_visible && row_idx == cursor_row && col_idx == cursor_col;
                if is_cursor {
                    flags |= match grid.cursor.style {
                        CursorStyle::Block => FLAG_CURSOR_BLOCK,
                        CursorStyle::Beam => FLAG_CURSOR_BEAM,
                        CursorStyle::Underline => FLAG_CURSOR_UNDERLINE,
                    };
                }

                // Skip completely blank default-background cells with no decorations or cursor.
                if cell.ch == ' ' && bg == config.default_bg && flags == 0 {
                    continue;
                }

                let (glyph_rect, uv_rect) = if cell.ch != ' ' && !cell.ch.is_control() {
                    let key = GlyphKey {
                        ch: cell.ch,
                        bold: cell.flags.bold,
                        italic: cell.flags.italic,
                    };
                    let entry = self.atlas.get_or_insert(font, key);
                    if entry.width > 0.0 && entry.height > 0.0 {
                        let gx = entry.xmin;
                        let gy = baseline - entry.height - entry.ymin;
                        ([gx, gy, entry.width, entry.height], entry.uv_rect)
                    } else {
                        ([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0])
                    }
                } else {
                    ([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0])
                };

                self.instances_scratch.push(CellInstance {
                    cell_rect: [cell_x, cell_y, cw, ch],
                    glyph_rect,
                    uv_rect,
                    fg_rgba: fg,
                    bg_rgba: bg,
                    flags,
                    cursor_rgba: config.cursor_color,
                });
            }
        }
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            for frame in &mut self.frames {
                self.device.unmap_memory(frame.instance_memory);
                self.device.destroy_buffer(frame.instance_buffer, None);
                self.device.free_memory(frame.instance_memory, None);
                self.device.destroy_fence(frame.in_flight, None);
                self.device.destroy_semaphore(frame.render_finished, None);
                self.device.destroy_semaphore(frame.image_available, None);
            }
            self.pipeline.destroy(&self.device);
            self.atlas.destroy(&self.device);
            self.swapchain.destroy_all(&self.device);
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

fn pick_physical_device(
    instance: &ash::Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), String> {
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|e| format!("enumerate_physical_devices failed: {e}"))?;

    let mut best: Option<(u32, vk::PhysicalDevice, u32)> = None;

    for pdev in devices {
        let props = unsafe { instance.get_physical_device_properties(pdev) };
        let queue_families = unsafe { instance.get_physical_device_queue_family_properties(pdev) };

        for (idx, qf) in queue_families.iter().enumerate() {
            if !qf.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                continue;
            }
            let supports_surface = unsafe {
                surface_loader.get_physical_device_surface_support(pdev, idx as u32, surface)
            }
            .unwrap_or(false);
            if !supports_surface {
                continue;
            }

            let score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 100,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 50,
                _ => 10,
            };
            if best.is_none_or(|(best_score, _, _)| score > best_score) {
                best = Some((score, pdev, idx as u32));
            }
            break;
        }
    }

    best.map(|(_, pdev, q)| (pdev, q))
        .ok_or_else(|| "no Vulkan GPU supporting Wayland surface graphics found".to_string())
}

fn create_instance_buffer(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    capacity: usize,
) -> Result<(vk::Buffer, vk::DeviceMemory, *mut CellInstance), String> {
    let byte_size = (capacity * std::mem::size_of::<CellInstance>()) as u64;
    let buf_info = vk::BufferCreateInfo::default()
        .size(byte_size)
        .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buf_info, None) }
        .map_err(|e| format!("create_buffer for instances failed: {e}"))?;
    let reqs = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem_type = find_memory_type(
        mem_props,
        reqs.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let memory = unsafe { device.allocate_memory(&alloc_info, None) }
        .map_err(|e| format!("allocate_memory for instances failed: {e}"))?;
    unsafe { device.bind_buffer_memory(buffer, memory, 0) }
        .map_err(|e| format!("bind_buffer_memory for instances failed: {e}"))?;

    let ptr = unsafe { device.map_memory(memory, 0, byte_size, vk::MemoryMapFlags::empty()) }
        .map_err(|e| format!("map_memory for instances failed: {e}"))?
        as *mut CellInstance;

    Ok((buffer, memory, ptr))
}

fn is_srgb_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB
            | vk::Format::R8G8B8A8_SRGB
            | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn dim_rgba(packed: u32) -> u32 {
    let r = (((packed >> 24) & 0xFF) * 2) / 3;
    let g = (((packed >> 16) & 0xFF) * 2) / 3;
    let b = (((packed >> 8) & 0xFF) * 2) / 3;
    let a = packed & 0xFF;
    (r << 24) | (g << 16) | (b << 8) | a
}
